use std::collections::BTreeSet;

use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use loong_kernel::access::memory::{MemoryReplaceTurnsOutcome, MemoryTurn};
use serde_json::Value;

use crate::config::LoongConfig;
use crate::{CliResult, Context};

#[cfg(feature = "memory-sqlite")]
use crate::memory;

#[cfg(feature = "memory-sqlite")]
use super::compaction::{compact_window, compaction_policy_from_config};
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
    pub(crate) runtime_self_continuity:
        Option<crate::runtime_self_continuity::RuntimeSelfContinuity>,
}

impl AssembledConversationContext {
    pub fn from_messages(messages: Vec<Value>) -> Self {
        Self {
            messages,
            artifacts: Vec::new(),
            estimated_tokens: None,
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
            runtime_self_continuity: None,
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

/// Builds and maintains conversation context under one bound Session authority.
///
/// `Context` is the sole source of the current Session identity. Lifecycle hooks
/// accept a child id only when that child is the operation's explicit target.
#[async_trait]
pub trait ConversationContextEngine: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> ContextEngineMetadata {
        ContextEngineMetadata::new(self.id(), [])
    }

    async fn bootstrap(
        &self,
        _config: &LoongConfig,
        _ctx: &Context<'_>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _message: &Value,
        _ctx: &Context<'_>,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
    }

    async fn after_turn(
        &self,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _subagent_session_id: &str,
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _subagent_session_id: &str,
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn assemble_context(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        ctx: &Context<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.assemble_messages(config, include_system_prompt, ctx)
            .await
            .map(AssembledConversationContext::from_messages)
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        ctx: &Context<'_>,
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
        ctx: &Context<'_>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.as_ref().bootstrap(config, ctx).await
    }

    async fn ingest(
        &self,
        message: &Value,
        ctx: &Context<'_>,
    ) -> CliResult<ContextEngineIngestResult> {
        self.as_ref().ingest(message, ctx).await
    }

    async fn after_turn(
        &self,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.as_ref()
            .after_turn(user_input, assistant_reply, messages, ctx)
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.as_ref().compact_context(config, messages, ctx).await
    }

    async fn prepare_subagent_spawn(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.as_ref()
            .prepare_subagent_spawn(subagent_session_id, ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.as_ref()
            .on_subagent_ended(subagent_session_id, ctx)
            .await
    }

    async fn assemble_context(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        ctx: &Context<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.as_ref()
            .assemble_context(config, include_system_prompt, ctx)
            .await
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        ctx: &Context<'_>,
    ) -> CliResult<Vec<Value>> {
        self.as_ref()
            .assemble_messages(config, include_system_prompt, ctx)
            .await
    }
}

/// Built-in context engine; memory authority comes exclusively from each call's Context.
pub struct DefaultContextEngine;

impl DefaultContextEngine {
    pub(crate) fn engine_metadata() -> ContextEngineMetadata {
        #[cfg(feature = "memory-sqlite")]
        let capabilities = [
            ContextEngineCapability::KernelMemoryWindowRead,
            ContextEngineCapability::ContextCompaction,
        ];
        #[cfg(not(feature = "memory-sqlite"))]
        let capabilities: [ContextEngineCapability; 0] = [];
        ContextEngineMetadata::new("default", capabilities)
    }

    #[cfg(feature = "memory-sqlite")]
    async fn persist_memory_window(
        &self,
        turns: &[memory::WindowTurn],
        expected_turn_count: Option<usize>,
        context: &Context<'_>,
    ) -> CliResult<PersistMemoryWindowOutcome> {
        let turns = turns
            .iter()
            .map(|turn| MemoryTurn {
                role: turn.role.clone(),
                content: turn.content.clone(),
                ts: turn.ts,
            })
            .collect();
        let outcome = context
            .access()
            .memory()
            .replace_turns(turns, expected_turn_count)
            .await
            .map_err(|error| format!("persist compacted memory window failed: {error}"))?;

        match outcome {
            MemoryReplaceTurnsOutcome::Replaced => Ok(PersistMemoryWindowOutcome::Persisted),
            MemoryReplaceTurnsOutcome::Conflict => Ok(PersistMemoryWindowOutcome::Conflict),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    async fn load_stage_envelope(
        &self,
        _config: &LoongConfig,
        context: &Context<'_>,
    ) -> CliResult<memory::StageEnvelope> {
        context
            .access()
            .memory()
            .read_stage_envelope()
            .await
            .map_err(|error| format!("load staged memory envelope failed: {error}"))
    }
}

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
        Self::engine_metadata()
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        _messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        #[cfg(feature = "memory-sqlite")]
        {
            const MAX_COMPACTION_CONFLICT_RETRIES: usize = 3;

            for _ in 0..MAX_COMPACTION_CONFLICT_RETRIES {
                let snapshot = self.load_compaction_session_snapshot(ctx).await?;
                if !snapshot.is_complete() {
                    return Ok(());
                }
                let Some(compact_policy) = compaction_policy_from_config(config) else {
                    return Ok(());
                };
                let Some(compacted) = compact_window(&snapshot.turns, compact_policy) else {
                    return Ok(());
                };

                match self
                    .persist_memory_window(&compacted, Some(snapshot.turn_count), ctx)
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
            let _ = (config, ctx);
            Ok(())
        }
    }

    async fn assemble_context(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        ctx: &Context<'_>,
    ) -> CliResult<AssembledConversationContext> {
        #[cfg(feature = "memory-sqlite")]
        {
            let envelope = self.load_stage_envelope(config, ctx).await?;
            let projected = crate::provider::project_stage_envelope_with_context(
                config,
                include_system_prompt,
                ctx,
                &envelope,
            )
            .await?;
            return Ok(AssembledConversationContext {
                messages: projected.messages,
                artifacts: projected.artifacts,
                estimated_tokens: None,
                prompt_fragments: projected.prompt_fragments,
                system_prompt_addition: None,
                runtime_self_continuity: projected.runtime_self_continuity,
            });
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            crate::provider::build_messages_for_session(config, include_system_prompt, ctx)
                .await
                .map(AssembledConversationContext::from_messages)
        }
    }

    async fn assemble_messages(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        ctx: &Context<'_>,
    ) -> CliResult<Vec<Value>> {
        self.assemble_context(config, include_system_prompt, ctx)
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
        include_system_prompt: bool,
        ctx: &Context<'_>,
    ) -> CliResult<Vec<Value>> {
        crate::provider::build_messages_for_session(config, include_system_prompt, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MemoryProfile;
    use crate::test_support::TurnTestHarness;

    #[cfg(feature = "memory-sqlite")]
    async fn provider_messages_with_context(
        config: &LoongConfig,
        ctx: &crate::Context<'_>,
    ) -> Vec<Value> {
        crate::provider::build_projected_context_for_session(config, true, ctx)
            .await
            .expect("build provider context")
            .messages
    }

    #[test]
    fn default_engine_metadata_has_stable_identity() {
        let metadata = DefaultContextEngine::engine_metadata();
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
    async fn default_engine_assembles_runtime_self_through_governed_access_path() {
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

        let ctx = harness.context();
        let messages = crate::conversation::DefaultContextEngine
            .assemble_messages(&config, true, &ctx)
            .await
            .expect("assemble messages");

        let system_content = messages[0]["content"].as_str().expect("system content");

        assert!(system_content.contains(agents_text));

        let audit_events = harness.audit.snapshot();
        let has_typed_tool_event = audit_events.iter().any(|event| {
            matches!(
                &event.kind,
                loong_kernel::AuditEventKind::ActionExecution { .. }
            )
        });
        let has_legacy_tool_plane_event = audit_events.iter().any(|event| {
            matches!(
                &event.kind,
                loong_kernel::AuditEventKind::PlaneInvoked {
                    plane: loong_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });

        assert!(
            !has_typed_tool_event,
            "runtime-source file reads are governed access, not tool invocations"
        );
        assert!(
            !has_legacy_tool_plane_event,
            "runtime self path reads should not fall back to legacy tool-plane audit"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_messages_match_provider_summary_projection() {
        let durable_flush_lock = crate::test_utils::durable_memory_flush_test_lock();
        let _guard = durable_flush_lock.lock().await;
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = harness.session.session_id().to_owned();
        let sqlite_path = harness.temp_dir.join("memory.sqlite3");
        let sqlite_path_text = sqlite_path.display().to_string();
        let mut config = LoongConfig::default();

        config.tools.file_root = Some(harness.temp_dir.display().to_string());
        config.memory.profile = MemoryProfile::WindowPlusSummary;
        config.memory.sliding_window = 2;
        config.memory.sqlite_path = sqlite_path_text.clone();

        let memory_config =
            crate::session::store::session_store_config_from_memory_config_without_env_overrides(
                &config.memory,
            );
        crate::test_support::ensure_root_session_for_test(&config, session_id.as_str())
            .expect("persist summary test Session identity");

        crate::session::store::append_session_turn_direct(
            session_id.as_str(),
            "user",
            "turn 1",
            &memory_config,
        )
        .expect("append turn 1 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id.as_str(),
            "assistant",
            "turn 2",
            &memory_config,
        )
        .expect("append turn 2 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id.as_str(),
            "user",
            "turn 3",
            &memory_config,
        )
        .expect("append turn 3 should succeed");
        crate::session::store::append_session_turn_direct(
            session_id.as_str(),
            "assistant",
            "turn 4",
            &memory_config,
        )
        .expect("append turn 4 should succeed");

        let session = crate::Session::from_config(
            harness.runtime.as_ref(),
            &config,
            session_id,
            harness.session.agent_id(),
            harness.session.session_mode,
        )
        .expect("materialize summary test Session from its final config");
        let ctx = crate::Context::new(harness.runtime.as_ref(), &session)
            .expect("summary test Session belongs to the harness Runtime");
        let kernel_messages = crate::conversation::DefaultContextEngine
            .assemble_messages(&config, true, &ctx)
            .await
            .expect("assemble messages");
        let provider_messages = provider_messages_with_context(&config, &ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "context-bound assembly should preserve summary projection parity"
        );
        assert!(
            kernel_messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("## Memory Summary"))
            }),
            "expected context-bound assembly to keep the summary block"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn default_engine_kernel_bound_messages_match_provider_profile_projection() {
        let durable_flush_lock = crate::test_utils::durable_memory_flush_test_lock();
        let _guard = durable_flush_lock.lock().await;
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let session_id = harness.session.session_id().to_owned();
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
            crate::session::store::session_store_config_from_memory_config_without_env_overrides(
                &config.memory,
            );
        crate::test_support::ensure_root_session_for_test(&config, session_id.as_str())
            .expect("persist profile test Session identity");

        crate::session::store::append_session_turn_direct(
            session_id.as_str(),
            "assistant",
            "turn 1",
            &memory_config,
        )
        .expect("append turn should succeed");

        let session = crate::Session::from_config(
            harness.runtime.as_ref(),
            &config,
            session_id,
            harness.session.agent_id(),
            harness.session.session_mode,
        )
        .expect("materialize profile test Session from its final config");
        let ctx = crate::Context::new(harness.runtime.as_ref(), &session)
            .expect("profile test Session belongs to the harness Runtime");
        let kernel_messages = crate::conversation::DefaultContextEngine
            .assemble_messages(&config, true, &ctx)
            .await
            .expect("assemble messages");
        let provider_messages = provider_messages_with_context(&config, &ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "context-bound assembly should preserve profile projection parity"
        );
        assert!(
            kernel_messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains(profile_note))
            }),
            "expected context-bound assembly to keep the profile block"
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
        let ctx = harness.context();
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

        let kernel_messages = crate::conversation::DefaultContextEngine
            .assemble_messages(&config, true, &ctx)
            .await
            .expect("assemble messages");
        let provider_messages = provider_messages_with_context(&config, &ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "context-bound assembly should preserve durable recall projection parity"
        );
        assert!(
            kernel_messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"].as_str().is_some_and(|content| {
                        content.contains("## Advisory Durable Recall")
                            && content.contains("Remember the deploy freeze window.")
                    })
            }),
            "expected context-bound assembly to keep the durable recall block"
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
        let ctx = harness.context();
        let session_id = ctx.session().session_id();
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
            crate::session::store::session_store_config_from_memory_config_without_env_overrides(
                &config.memory,
            );
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

        let assembled = crate::conversation::DefaultContextEngine
            .assemble_context(&config, true, &ctx)
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
    #[allow(clippy::await_holding_lock)] // Both projections reread process env; serialize with env-mutating tests.
    async fn default_engine_kernel_bound_messages_match_provider_governed_profile_projection() {
        let _env_guard = crate::test_support::lock_process_env_for_tests();
        let durable_flush_lock = crate::test_utils::durable_memory_flush_test_lock();
        let _guard = durable_flush_lock.lock().await;
        let capabilities = std::collections::BTreeSet::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
            loong_contracts::Capability::FilesystemWrite,
            loong_contracts::Capability::MemoryRead,
        ]);
        let harness = TurnTestHarness::with_capabilities(capabilities);
        let ctx = harness.context();
        let session_id = ctx.session().session_id();
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

        let kernel_messages = crate::conversation::DefaultContextEngine
            .assemble_messages(&config, true, &ctx)
            .await
            .expect("assemble messages");
        let provider_messages = provider_messages_with_context(&config, &ctx).await;

        assert_eq!(
            kernel_messages, provider_messages,
            "context-bound assembly should preserve governed profile projection parity"
        );
    }
}
