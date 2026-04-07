use super::*;
use crate::config::ToolConfig;
use crate::context::bootstrap_test_kernel_context;
use crate::conversation::turn_engine::ToolBatchExecutionIntentTrace;
use crate::conversation::{
    ConversationTurnObserver, ConversationTurnPhase, ConversationTurnToolState,
};
use crate::session::repository::FinalizeSessionTerminalResult;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_direct_binding_reports_no_kernel_context() {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "file.read",
        json!({
            "path": "README.md",
        }),
        Some("root-session"),
        Some("turn-direct-core"),
    );
    let intent = ToolIntent {
        tool_name,
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-direct-core".to_owned(),
        tool_call_id: "call-direct-core".to_owned(),
    };
    let session_context =
        SessionContext::root_with_tool_view("root-session", crate::tools::planned_root_tool_view());
    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &crate::conversation::NoopAppToolDispatcher,
        ConversationRuntimeBinding::direct(),
        None,
        2_048,
    )
    .await
    .expect_err("direct core execution should fail closed without kernel context");

    assert_eq!(error.kind, PlanNodeErrorKind::PolicyDenied);
    assert_eq!(error.message, "no_kernel_context");
}

fn unique_sqlite_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loongclaw-turn-coordinator-{label}-{}.sqlite3",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[test]
fn pending_approval_input_parser_accepts_keyword_and_numeric_aliases() {
    assert_eq!(
        parse_pending_approval_input_decision("yes"),
        Some(PendingApprovalInputDecision::RunOnce)
    );
    assert_eq!(
        parse_pending_approval_input_decision("2"),
        Some(PendingApprovalInputDecision::SessionAuto)
    );
    assert_eq!(
        parse_pending_approval_input_decision("本会话全自动"),
        Some(PendingApprovalInputDecision::SessionFull)
    );
    assert_eq!(
        parse_pending_approval_input_decision("esc"),
        Some(PendingApprovalInputDecision::Cancel)
    );
    assert_eq!(
        parse_pending_approval_input_decision("跳过这次"),
        Some(PendingApprovalInputDecision::Cancel)
    );
    assert_eq!(
        parse_pending_approval_input_decision("skip call"),
        Some(PendingApprovalInputDecision::Cancel)
    );
    assert_eq!(parse_pending_approval_input_decision("maybe"), None);
}

#[cfg(feature = "memory-sqlite")]
#[derive(Default)]
struct ApprovalControlRuntime {
    bootstrap_calls: StdMutex<usize>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for ApprovalControlRuntime {
    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "approval control test"
        })])
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok("approval handled".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn should not run during approval control replay")
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn_streaming should not run during approval control replay")
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<crate::conversation::context_engine::ContextEngineBootstrapResult> {
        let mut bootstrap_calls = self
            .bootstrap_calls
            .lock()
            .expect("bootstrap call lock should not be poisoned");
        *bootstrap_calls += 1;
        Ok(Default::default())
    }
}

#[cfg(feature = "memory-sqlite")]
struct CoreReplayRuntime;

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for CoreReplayRuntime {
    fn session_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        Err("session_context should not be called for core approval replay".to_owned())
    }

    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Err("build_messages should not run during core approval replay".to_owned())
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Err("request_completion should not run during core approval replay".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        Err("request_turn should not run during core approval replay".to_owned())
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        Err("request_turn_streaming should not run during core approval replay".to_owned())
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Err("persist_turn should not run during core approval replay".to_owned())
    }
}

#[cfg(feature = "memory-sqlite")]
fn sqlite_memory_config(label: &str) -> MemoryRuntimeConfig {
    let path = unique_sqlite_path(label);
    let _ = std::fs::remove_file(&path);
    let mut config = LoongClawConfig::default();
    config.memory.sqlite_path = path.display().to_string();
    MemoryRuntimeConfig::from_memory_config(&config.memory)
}

#[cfg(feature = "memory-sqlite")]
fn seed_pending_approval_request(
    repo: &SessionRepository,
    session_id: &str,
    approval_request_id: &str,
    tool_name: &str,
    execution_kind: &str,
) {
    repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
        approval_request_id: approval_request_id.to_owned(),
        session_id: session_id.to_owned(),
        turn_id: "turn-pending-approval".to_owned(),
        tool_call_id: "call-pending-approval".to_owned(),
        tool_name: tool_name.to_owned(),
        approval_key: format!("tool:{tool_name}"),
        request_payload_json: json!({
            "session_id": session_id,
            "turn_id": "turn-pending-approval",
            "tool_call_id": "call-pending-approval",
            "tool_name": tool_name,
            "args_json": {},
            "source": "test",
            "execution_kind": execution_kind,
        }),
        governance_snapshot_json: json!({
            "rule_id": "governed_tool_requires_approval",
        }),
    })
    .expect("seed approval request");
}

#[cfg(feature = "memory-sqlite")]
fn unique_workspace_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loongclaw-turn-coordinator-workspace-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[cfg(feature = "memory-sqlite")]
#[derive(Default)]
struct RecordingCompactRuntime {
    compact_calls: StdMutex<usize>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for RecordingCompactRuntime {
    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok(String::new())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn should not be called in compaction tests")
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn_streaming should not be called in compaction tests")
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
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
        let mut compact_calls = self.compact_calls.lock().expect("compact lock");
        *compact_calls += 1;
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
struct CompactSessionBuildMessagesRuntime {
    session_tool_view: crate::tools::ToolView,
    build_messages_calls: StdMutex<Vec<(bool, crate::tools::ToolView)>>,
    fail_after_first_readback: bool,
}

#[cfg(feature = "memory-sqlite")]
impl CompactSessionBuildMessagesRuntime {
    fn new(session_tool_view: crate::tools::ToolView, fail_after_first_readback: bool) -> Self {
        Self {
            session_tool_view,
            build_messages_calls: StdMutex::new(Vec::new()),
            fail_after_first_readback,
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for CompactSessionBuildMessagesRuntime {
    fn session_context(
        &self,
        _config: &LoongClawConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        Ok(SessionContext::root_with_tool_view(
            session_id,
            self.session_tool_view.clone(),
        ))
    }

    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        include_system_prompt: bool,
        tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        let mut build_messages_calls = self
            .build_messages_calls
            .lock()
            .expect("build_messages lock should not be poisoned");
        build_messages_calls.push((include_system_prompt, tool_view.clone()));
        let call_count = build_messages_calls.len();

        if self.fail_after_first_readback && call_count > 1 {
            return Err("post-compaction readback failed".to_owned());
        }

        let tool_names = tool_view.tool_names().collect::<Vec<_>>();
        let tool_names = tool_names.join(",");

        Ok(vec![json!({
            "role": "system",
            "content": format!(
                "include_system_prompt={include_system_prompt} tools={tool_names}"
            ),
        })])
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok(String::new())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn should not be called in compact_session tests")
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn_streaming should not be called in compact_session tests")
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
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
}

#[derive(Default)]
struct ObserverStreamingRuntime {
    streaming_calls: StdMutex<usize>,
}

#[async_trait]
impl ConversationRuntime for ObserverStreamingRuntime {
    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "stay focused"
        })])
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok("completion".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn should not be called when observer streaming is enabled")
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        let mut streaming_calls = self
            .streaming_calls
            .lock()
            .expect("streaming call lock should not be poisoned");
        *streaming_calls += 1;

        if let Some(on_token) = on_token {
            on_token(crate::provider::StreamingCallbackData::Text {
                text: "draft".to_owned(),
            });
        }

        Ok(ProviderTurn {
            assistant_text: "final reply".to_owned(),
            tool_intents: Vec::new(),
            raw_meta: Value::Null,
        })
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[derive(Default)]
struct ObserverFallbackRuntime {
    request_turn_calls: StdMutex<usize>,
    request_turn_streaming_calls: StdMutex<usize>,
}

#[async_trait]
impl ConversationRuntime for ObserverFallbackRuntime {
    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "stay focused"
        })])
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok("completion".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        let mut request_turn_calls = self
            .request_turn_calls
            .lock()
            .expect("request-turn call lock should not be poisoned");
        *request_turn_calls += 1;

        Ok(ProviderTurn {
            assistant_text: "final reply".to_owned(),
            tool_intents: Vec::new(),
            raw_meta: Value::Null,
        })
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        let mut request_turn_streaming_calls = self
            .request_turn_streaming_calls
            .lock()
            .expect("request-turn-streaming call lock should not be poisoned");
        *request_turn_streaming_calls += 1;
        panic!("request_turn_streaming should not be called for unsupported transports")
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingTurnObserver {
    phase_events: StdMutex<Vec<ConversationTurnPhaseEvent>>,
    tool_events: StdMutex<Vec<ConversationTurnToolEvent>>,
    token_events: StdMutex<Vec<crate::acp::StreamingTokenEvent>>,
}

impl ConversationTurnObserver for RecordingTurnObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        let mut phase_events = self
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        phase_events.push(event);
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        let mut tool_events = self
            .tool_events
            .lock()
            .expect("tool event lock should not be poisoned");
        tool_events.push(event);
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        let mut token_events = self
            .token_events
            .lock()
            .expect("token event lock should not be poisoned");
        token_events.push(event);
    }
}

#[tokio::test]
async fn handle_turn_with_observer_uses_streaming_request_and_emits_live_events() {
    let mut config = LoongClawConfig::default();
    config.provider.kind = crate::config::ProviderKind::Anthropic;

    let runtime = ObserverStreamingRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
        )
        .await
        .expect("observer turn should succeed");

    assert_eq!(reply, "final reply");

    let streaming_calls = runtime
        .streaming_calls
        .lock()
        .expect("streaming call lock should not be poisoned");
    assert_eq!(*streaming_calls, 1);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::ContextReady,
            ConversationTurnPhase::RequestingProvider,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );

    let token_events = observer
        .token_events
        .lock()
        .expect("token event lock should not be poisoned");
    assert_eq!(token_events.len(), 1);
    assert_eq!(token_events[0].event_type, "text_delta");
    assert_eq!(token_events[0].delta.text.as_deref(), Some("draft"));
}

#[tokio::test]
async fn handle_turn_with_observer_falls_back_when_streaming_events_are_unsupported() {
    let mut config = LoongClawConfig::default();
    config.provider.kind = crate::config::ProviderKind::Openai;

    let runtime = ObserverFallbackRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
        )
        .await
        .expect("observer turn should succeed");

    assert_eq!(reply, "final reply");

    let request_turn_calls = runtime
        .request_turn_calls
        .lock()
        .expect("request-turn call lock should not be poisoned");
    assert_eq!(*request_turn_calls, 1);

    let request_turn_streaming_calls = runtime
        .request_turn_streaming_calls
        .lock()
        .expect("request-turn-streaming call lock should not be poisoned");
    assert_eq!(*request_turn_streaming_calls, 0);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::ContextReady,
            ConversationTurnPhase::RequestingProvider,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );

    let token_events = observer
        .token_events
        .lock()
        .expect("token event lock should not be poisoned");
    assert!(
        token_events.is_empty(),
        "unsupported transports should not emit streaming token events: {token_events:#?}"
    );
}

#[tokio::test]
async fn handle_turn_with_observer_emits_lifecycle_for_explicit_acp_inline_message() {
    let config = LoongClawConfig::default();
    let runtime = ObserverStreamingRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::explicit();
    let address = ConversationSessionAddress::from_session_id("observer-session");
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::InlineMessage,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
        )
        .await
        .expect("ACP inline reply should succeed");

    let expected_reply =
        format_provider_error_reply("ACP is disabled by policy (`acp.enabled=false`)");
    assert_eq!(reply, expected_reply);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );

    let tool_events = observer
        .tool_events
        .lock()
        .expect("tool event lock should not be poisoned");
    assert!(tool_events.is_empty());

    let token_events = observer
        .token_events
        .lock()
        .expect("token event lock should not be poisoned");
    assert!(token_events.is_empty());

    let streaming_calls = runtime
        .streaming_calls
        .lock()
        .expect("streaming call lock should not be poisoned");
    assert_eq!(*streaming_calls, 0);
}

#[tokio::test]
async fn handle_turn_with_ingress_and_observer_marks_failed_when_runtime_bootstrap_fails() {
    let mut config = LoongClawConfig::default();
    config.conversation.context_engine = Some("missing-observer-runtime-ingress".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");

    let result = coordinator
        .handle_turn_with_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
        )
        .await;
    let _error = result.expect_err("missing runtime bootstrap should fail");

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
}

#[tokio::test]
async fn handle_turn_with_observer_marks_failed_when_runtime_bootstrap_fails() {
    let mut config = LoongClawConfig::default();
    config.conversation.context_engine = Some("missing-observer-runtime".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");

    let result = coordinator
        .handle_turn_with_address_and_acp_options_and_observer(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            Some(observer_handle),
        )
        .await;
    let _error = result.expect_err("missing runtime bootstrap should fail");

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
}

#[test]
fn build_provider_turn_tool_terminal_events_prefers_trace_outcomes_over_generic_fallbacks() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "sessions_list".to_owned(),
                args_json: json!({}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "session_status".to_owned(),
                args_json: json!({"session_id": "session-a"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };
    let turn_result = TurnResult::ToolError(TurnFailure::retryable(
        "tool_execution_failed",
        "second tool failed",
    ));
    let trace = ToolBatchExecutionTrace {
        total_intents: 2,
        parallel_execution_enabled: false,
        parallel_execution_max_in_flight: 1,
        observed_peak_in_flight: 1,
        observed_wall_time_ms: 10,
        segments: Vec::new(),
        decision_records: Vec::new(),
        outcome_records: Vec::new(),
        intent_outcomes: vec![
            ToolBatchExecutionIntentTrace {
                tool_call_id: "call-1".to_owned(),
                tool_name: "sessions_list".to_owned(),
                status: ToolBatchExecutionIntentStatus::Completed,
                detail: None,
            },
            ToolBatchExecutionIntentTrace {
                tool_call_id: "call-2".to_owned(),
                tool_name: "session_status".to_owned(),
                status: ToolBatchExecutionIntentStatus::Failed,
                detail: Some("second tool failed".to_owned()),
            },
        ],
    };

    let events = build_provider_turn_tool_terminal_events(&turn, &turn_result, Some(&trace));

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].tool_call_id, "call-1");
    assert_eq!(events[0].state, ConversationTurnToolState::Completed);
    assert_eq!(events[1].tool_call_id, "call-2");
    assert_eq!(events[1].state, ConversationTurnToolState::Failed);
    assert_eq!(events[1].detail.as_deref(), Some("second tool failed"));
}

#[test]
fn build_provider_turn_tool_terminal_events_attach_canonical_shell_request_summary() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: json!({"command": "ls /root"}),
            source: "provider_tool_call".to_owned(),
            session_id: "session-a".to_owned(),
            turn_id: "turn-a".to_owned(),
            tool_call_id: "call-shell".to_owned(),
        }],
        raw_meta: Value::Null,
    };
    let turn_result = TurnResult::ToolDenied(TurnFailure::policy_denied(
        "shell_policy_denied",
        "policy_denied: command contains embedded whitespace",
    ));

    let events = build_provider_turn_tool_terminal_events(&turn, &turn_result, None);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tool_call_id, "call-shell");
    assert_eq!(events[0].state, ConversationTurnToolState::Denied);
    let request_summary =
        summarize_tool_event_request(&turn.tool_intents[0]).expect("request summary");
    let request_summary_json: Value =
        serde_json::from_str(&request_summary).expect("request summary should be valid json");
    assert_eq!(
        request_summary_json,
        json!({
            "tool": "shell.exec",
            "request": {"command": "ls", "args_redacted": 1}
        })
    );
}

#[test]
fn summarize_failed_provider_lane_tool_request_preserves_multi_intent_context_without_trace() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "Cargo.toml"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "shell.exec".to_owned(),
                args_json: json!({"command": "ls /root"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let request_summary = summarize_provider_lane_tool_request(
        &turn,
        &TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure")),
        None,
    )
    .expect("multi-intent failures should retain a request summary");
    let request_summary_json: Value =
        serde_json::from_str(&request_summary).expect("request summary should be valid json");
    let request_entries = request_summary_json
        .as_array()
        .expect("multi-intent request summary should be an array");

    assert_eq!(request_entries.len(), 2);
    assert_eq!(request_entries[0]["tool"], "file.read");
    assert_eq!(request_entries[1]["tool"], "shell.exec");
    assert_eq!(request_entries[1]["request"]["command"], "ls");
    assert_eq!(request_entries[1]["request"]["args_redacted"], 1);
}
#[cfg(feature = "memory-sqlite")]
fn finalize_recovered_child(
    repo: &SessionRepository,
    expected_state: SessionState,
) -> FinalizeSessionTerminalResult {
    repo.finalize_session_terminal_if_current(
        "child-session",
        expected_state,
        FinalizeSessionTerminalRequest {
            state: SessionState::Failed,
            last_error: Some("delegate_recovered".to_owned()),
            event_kind: RECOVERY_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "recovery_kind": "forced_recovery",
                "recovered_state": "failed",
            }),
            outcome_status: "error".to_owned(),
            outcome_payload_json: json!({
                "error": "delegate_recovered"
            }),
        },
    )
    .expect("recover child terminal state")
    .expect("recovery should transition child")
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_delegate_child_terminal_with_recovery_does_not_overwrite_recovered_failure() {
    let memory_config = sqlite_memory_config("recovered-running-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child session");

    let recovered = finalize_recovered_child(&repo, SessionState::Running);
    assert_eq!(recovered.session.state, SessionState::Failed);
    assert_eq!(recovered.terminal_outcome.status, "error");

    finalize_delegate_child_terminal_with_recovery(
        &repo,
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "turn_count": 1,
                "duration_ms": 12,
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "child-session",
                "final_output": "late success",
            }),
        },
    )
    .expect("stale running finalizer should no-op");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Failed);
    assert_eq!(child.last_error.as_deref(), Some("delegate_recovered"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&RECOVERY_EVENT_KIND));
    assert!(!event_kinds.contains(&"delegate_completed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(terminal_outcome.payload_json["error"], "delegate_recovered");
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_async_delegate_spawn_failure_does_not_overwrite_recovered_failure() {
    let memory_config = sqlite_memory_config("recovered-ready-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let execution = ConstrainedSubagentExecution {
        mode: ConstrainedSubagentMode::Async,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        depth: 1,
        max_depth: 1,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec![
            "file.read".to_owned(),
            "file.write".to_owned(),
            "file.edit".to_owned(),
        ],
        workspace_root: None,
        runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
        kernel_bound: false,
        identity: None,
        profile: Some(crate::conversation::ConstrainedSubagentProfile::for_child_depth(1, 1)),
    };
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");

    let recovered = finalize_recovered_child(&repo, SessionState::Ready);
    assert_eq!(recovered.session.state, SessionState::Failed);
    assert_eq!(recovered.terminal_outcome.status, "error");

    finalize_async_delegate_spawn_failure(
        &memory_config,
        "child-session",
        "root-session",
        Some("Child".to_owned()),
        None,
        &execution,
        "spawn unavailable".to_owned(),
    )
    .expect("stale queued spawn failure finalizer should no-op");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Failed);
    assert_eq!(child.last_error.as_deref(), Some("delegate_recovered"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&RECOVERY_EVENT_KIND));
    assert!(!event_kinds.contains(&"delegate_spawn_failed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(terminal_outcome.payload_json["error"], "delegate_recovered");
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_delegate_child_terminal_with_recovery_errors_when_child_session_missing() {
    let memory_config = sqlite_memory_config("missing-running-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let error = finalize_delegate_child_terminal_with_recovery(
        &repo,
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "turn_count": 1,
                "duration_ms": 12,
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "child-session",
                "final_output": "late success",
            }),
        },
    )
    .expect_err("missing child session should not be treated as stale");

    assert!(error.contains("session `child-session` not found"));
    assert!(error.contains("delegate_terminal_recovery_skipped_from_state: missing"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_async_delegate_spawn_failure_with_recovery_errors_when_child_session_missing() {
    let memory_config = sqlite_memory_config("missing-ready-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let execution = ConstrainedSubagentExecution {
        mode: ConstrainedSubagentMode::Async,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        depth: 1,
        max_depth: 1,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec![
            "file.read".to_owned(),
            "file.write".to_owned(),
            "file.edit".to_owned(),
        ],
        workspace_root: None,
        runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
        kernel_bound: false,
        identity: None,
        profile: Some(crate::conversation::ConstrainedSubagentProfile::for_child_depth(1, 1)),
    };
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let error = finalize_async_delegate_spawn_failure_with_recovery(
        &memory_config,
        "child-session",
        "root-session",
        Some("Child".to_owned()),
        None,
        &execution,
        "spawn unavailable".to_owned(),
    )
    .expect_err("missing child session should not bypass spawn failure recovery");

    assert!(error.contains("session `child-session` not found"));
    assert!(error.contains("delegate_async_spawn_recovery_skipped_from_state: missing"));
    assert_eq!(
        repo.load_session("child-session")
            .expect("load child session"),
        None
    );
}

#[test]
fn build_turn_reply_followup_messages_include_truncation_hint_for_truncated_tool_results() {
    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#.to_owned(),
        },
        "summarize note.md",
    );

    let user_prompt = messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Original request:\nsummarize note.md"));
}

#[test]
fn build_turn_reply_followup_messages_do_not_include_truncation_hint_for_failure() {
    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
        },
        "summarize note.md",
    );

    let user_prompt = messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(!user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT));
}

#[test]
fn build_turn_reply_followup_messages_promotes_external_skill_invoke_to_system_context() {
    let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
            },
            "summarize note.md",
        );

    assert!(
        messages.iter().any(|message| message.get("role")
            == Some(&Value::String("system".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content
                    .contains("Follow the managed skill instruction before answering."))
                .unwrap_or(false)),
        "safe-lane followup should promote invoked external skill instructions into system context: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .filter(|message| message.get("role") == Some(&Value::String("assistant".to_owned())))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .all(|content| !content.contains("[tool_result]\n[ok]")),
        "safe-lane followup should not carry invoke payload forward as an ordinary assistant tool_result: {messages:?}"
    );
}

#[test]
fn build_turn_reply_followup_messages_rejects_truncated_external_skill_invoke_payload() {
    let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":true}"#.to_owned(),
            },
            "summarize note.md",
        );

    assert!(
        !messages.iter().any(|message| message.get("role")
            == Some(&Value::String("system".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content
                    .contains("Follow the managed skill instruction before answering."))
                .unwrap_or(false)),
        "truncated invoke payload must not activate managed skill system context: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .filter(|message| message.get("role") == Some(&Value::String("assistant".to_owned())))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .any(|content| content.contains("[tool_result]\n[ok]")),
        "truncated invoke payload should stay as ordinary assistant tool_result content: {messages:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn persist_runtime_self_continuity_for_compaction_merges_live_and_stored_delegate_continuity() {
    let workspace_root = unique_workspace_root("merged-runtime-self-continuity");
    let memory_config = sqlite_memory_config("merged-runtime-self-continuity");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let root_session_id = "root-session";
    let child_session_id = "delegate:child-session";
    let live_agents_text = "Keep standing instructions visible.";
    let stored_identity_text = "# Identity\n\n- Name: Stored continuity identity";
    let mut config = LoongClawConfig::default();

    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), live_agents_text).expect("write AGENTS");
    config.memory.sqlite_path = sqlite_path;
    config.tools.file_root = Some(workspace_root.display().to_string());

    repo.create_session(NewSessionRecord {
        session_id: root_session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: child_session_id.to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some(root_session_id.to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");

    let stored_continuity = runtime_self_continuity::RuntimeSelfContinuity {
        runtime_self: crate::runtime_self::RuntimeSelfModel {
            identity_context: vec![stored_identity_text.to_owned()],
            ..Default::default()
        },
        resolved_identity: Some(crate::runtime_identity::ResolvedRuntimeIdentity {
            source: crate::runtime_identity::RuntimeIdentitySource::LegacyProfileNoteImport,
            content: stored_identity_text.to_owned(),
        }),
        session_profile_projection: None,
    };
    repo.append_event(NewSessionEvent {
        session_id: child_session_id.to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some(root_session_id.to_owned()),
        payload_json: json!({
            "runtime_self_continuity": stored_continuity,
        }),
    })
    .expect("append delegate event");

    persist_runtime_self_continuity_for_compaction(&config, child_session_id)
        .expect("persist merged runtime self continuity");

    let recent_events = repo
        .list_recent_events(child_session_id, 10)
        .expect("list recent events");
    let persisted_event = recent_events
        .iter()
        .rev()
        .find(|event| {
            event.event_kind == runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
        })
        .expect("persisted continuity event");
    let persisted_continuity = runtime_self_continuity::runtime_self_continuity_from_event_payload(
        &persisted_event.payload_json,
    )
    .expect("decode persisted continuity payload");

    assert_eq!(
        persisted_continuity.runtime_self.standing_instructions,
        vec![live_agents_text.to_owned()]
    );
    assert_eq!(
        persisted_continuity.runtime_self.identity_context,
        vec![stored_identity_text.to_owned()]
    );
    assert_eq!(
        persisted_continuity
            .resolved_identity
            .as_ref()
            .map(|value| value.content.as_str()),
        Some(stored_identity_text)
    );

    let _ = std::fs::remove_dir_all(&workspace_root);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn persist_runtime_self_continuity_for_compaction_reconstructs_legacy_delegate_session_row() {
    let workspace_root = unique_workspace_root("legacy-delegate-session-row");
    let memory_config = sqlite_memory_config("legacy-delegate-session-row");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let root_session_id = "root-session";
    let child_session_id = "delegate:legacy-child";
    let mut config = LoongClawConfig::default();

    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .clone();
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("AGENTS.md"),
        "Keep continuity explicit.",
    )
    .expect("write AGENTS");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root.display().to_string());

    repo.create_session(NewSessionRecord {
        session_id: root_session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite connection");
    conn.execute(
        "INSERT INTO turns(session_id, session_turn_index, role, content, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![child_session_id, 1_i64, "assistant", "legacy turn", 1_i64],
    )
    .expect("insert legacy turn");
    conn.execute(
        "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            child_session_id,
            "delegate_started",
            root_session_id,
            json!({}).to_string(),
            2_i64
        ],
    )
    .expect("insert legacy delegate event");
    drop(conn);

    persist_runtime_self_continuity_for_compaction(&config, child_session_id)
        .expect("persist runtime self continuity");

    let reconstructed_session = repo
        .load_session(child_session_id)
        .expect("load reconstructed session")
        .expect("reconstructed session row");

    assert_eq!(reconstructed_session.kind, SessionKind::DelegateChild);
    assert_eq!(
        reconstructed_session.parent_session_id.as_deref(),
        Some(root_session_id)
    );

    let _ = std::fs::remove_dir_all(&workspace_root);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn maybe_compact_context_fails_open_when_runtime_self_continuity_persist_cannot_reconstruct_delegate_lineage()
 {
    let workspace_root = unique_workspace_root("compaction-fail-open");
    let sqlite_path = unique_sqlite_path("compaction-fail-open");
    let runtime = RecordingCompactRuntime::default();
    let mut config = LoongClawConfig::default();

    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("AGENTS.md"),
        "Keep continuity explicit.",
    )
    .expect("write AGENTS");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root.display().to_string());
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = true;

    let kernel_ctx = bootstrap_test_kernel_context("turn-coordinator-compaction", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let runtime_handle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "trigger compaction"}),
    ];
    let outcome = runtime_handle.block_on(maybe_compact_context(
        &config,
        &runtime,
        "delegate:missing-lineage",
        &messages,
        Some(16),
        binding,
        false,
    ));

    assert_eq!(
        outcome.expect("compaction should fail open"),
        ContextCompactionOutcome::FailedOpen
    );
    let compact_calls = runtime.compact_calls.lock().expect("compact lock");
    assert_eq!(*compact_calls, 0);

    let _ = std::fs::remove_dir_all(&workspace_root);
    let _ = std::fs::remove_file(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn maybe_compact_context_fails_open_when_durable_flush_cannot_write_workspace_export() {
    let workspace_root_parent = unique_workspace_root("compaction-durable-flush-fail-open");
    let workspace_root_file = workspace_root_parent.join("workspace-root-file");
    let sqlite_path = unique_sqlite_path("compaction-durable-flush-fail-open");
    let runtime = RecordingCompactRuntime::default();
    let mut config = LoongClawConfig::default();

    std::fs::create_dir_all(&workspace_root_parent).expect("create workspace root parent");
    std::fs::write(
        workspace_root_parent.join("AGENTS.md"),
        "Keep continuity explicit.",
    )
    .expect("write AGENTS");
    std::fs::write(&workspace_root_file, "not a workspace directory")
        .expect("write workspace root file");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root_file.display().to_string());
    config.memory.sliding_window = 1;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = true;

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(
        "session-durable-flush-fail-open",
        "user",
        "remember the deployment cutoff",
        &memory_config,
    )
    .expect("append user turn");
    crate::memory::append_turn_direct(
        "session-durable-flush-fail-open",
        "assistant",
        "deployment cutoff is tonight",
        &memory_config,
    )
    .expect("append assistant turn");

    let kernel_ctx = bootstrap_test_kernel_context("turn-coordinator-compaction", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let runtime_handle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "trigger compaction"}),
    ];
    let outcome = runtime_handle.block_on(maybe_compact_context(
        &config,
        &runtime,
        "session-durable-flush-fail-open",
        &messages,
        Some(16),
        binding,
        false,
    ));

    assert_eq!(
        outcome.expect("compaction should fail open"),
        ContextCompactionOutcome::FailedOpen
    );
    let compact_calls = runtime.compact_calls.lock().expect("compact lock");
    assert_eq!(*compact_calls, 0);

    let _ = std::fs::remove_dir_all(&workspace_root_parent);
    let _ = std::fs::remove_file(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn compact_session_uses_session_context_tool_view_and_turn_like_build_flags() {
    let mut config = LoongClawConfig::default();
    let sqlite_path = unique_sqlite_path("compact-session-build-messages");
    let _ = std::fs::remove_file(&sqlite_path);
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(
        "compact-session-build-messages",
        "user",
        "remember this detail",
        &memory_config,
    )
    .expect("append user turn");

    let expected_tool_view = crate::tools::ToolView::from_tool_names(["status.inspect"]);
    let runtime = CompactSessionBuildMessagesRuntime::new(expected_tool_view.clone(), false);
    let kernel_ctx = bootstrap_test_kernel_context("compact-session-build-messages", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let coordinator = ConversationTurnCoordinator::new();

    let report = coordinator
        .compact_session_with_runtime(&config, "compact-session-build-messages", &runtime, binding)
        .await
        .expect("manual compaction should succeed");

    assert!(report.was_skipped());

    let build_messages_calls = runtime
        .build_messages_calls
        .lock()
        .expect("build_messages lock should not be poisoned");
    assert_eq!(build_messages_calls.len(), 2);
    assert!(
        build_messages_calls
            .iter()
            .all(|(include_system_prompt, _tool_view)| *include_system_prompt),
        "compact_session should mirror turn assembly by keeping the system prompt enabled"
    );
    assert!(
        build_messages_calls
            .iter()
            .all(|(_include_system_prompt, tool_view)| { *tool_view == expected_tool_view }),
        "compact_session should reuse the session-context tool view for both snapshots"
    );

    let _ = std::fs::remove_file(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn compact_session_skips_when_post_compaction_readback_fails() {
    let mut config = LoongClawConfig::default();
    let sqlite_path = unique_sqlite_path("compact-session-readback-fail");
    let _ = std::fs::remove_file(&sqlite_path);
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(
        "compact-session-readback-fail",
        "user",
        "keep the context intact",
        &memory_config,
    )
    .expect("append user turn");

    let runtime = CompactSessionBuildMessagesRuntime::new(
        crate::tools::ToolView::from_tool_names(["status.inspect"]),
        true,
    );
    let kernel_ctx = bootstrap_test_kernel_context("compact-session-readback-fail", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let coordinator = ConversationTurnCoordinator::new();

    let report = coordinator
        .compact_session_with_runtime(&config, "compact-session-readback-fail", &runtime, binding)
        .await
        .expect("manual compaction should degrade to skipped");

    assert!(report.was_skipped());
    assert_eq!(
        report.estimated_tokens_after,
        report.estimated_tokens_before
    );

    let build_messages_calls = runtime
        .build_messages_calls
        .lock()
        .expect("build_messages lock should not be poisoned");
    assert_eq!(build_messages_calls.len(), 2);

    let _ = std::fs::remove_file(&sqlite_path);
}

#[test]
fn build_turn_reply_followup_messages_reduces_file_read_payload_summary() {
    let content = (0..96)
        .map(|index| format!("line {index}: {}", "x".repeat(48)))
        .collect::<Vec<_>>()
        .join("\n");
    let payload_summary = serde_json::json!({
        "adapter": "core-tools",
        "tool_name": "file.read",
        "path": "/repo/README.md",
        "bytes": 8_192,
        "truncated": false,
        "content": content,
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-file",
            "payload_summary": payload_summary,
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "summarize README.md",
    );

    let assistant_tool_result = messages
        .iter()
        .find(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("assistant tool_result followup message should exist");
    let line = assistant_tool_result
        .lines()
        .nth(1)
        .expect("assistant tool_result should keep payload line");
    let envelope: Value = serde_json::from_str(
        line.strip_prefix("[ok] ")
            .expect("tool result line should preserve status prefix"),
    )
    .expect("reduced followup envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("file.read payload summary should stay valid json");

    assert_eq!(envelope["tool"], "file.read");
    assert_eq!(envelope["payload_truncated"], true);
    assert_eq!(summary["path"], "/repo/README.md");
    assert_eq!(summary["bytes"], 8_192);
    assert_eq!(summary["truncated"], false);
    assert!(summary.get("content_preview").is_some());
    assert!(summary.get("content_chars").is_some());
    assert_eq!(summary["content_truncated"], true);
}

#[test]
fn build_turn_reply_followup_messages_reduces_shell_exec_payload_summary() {
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "shell.exec",
            "tool_call_id": "call-shell",
            "payload_summary": serde_json::json!({
                "adapter": "core-tools",
                "tool_name": "shell.exec",
                "command": "cargo",
                "args": ["test", "--workspace"],
                "cwd": "/repo",
                "exit_code": 0,
                "stdout": (0..80)
                    .map(|index| format!("stdout line {index}: {}", "x".repeat(40)))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "stderr": (0..48)
                    .map(|index| format!("stderr line {index}: {}", "e".repeat(32)))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .to_string(),
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "summarize the test run",
    );

    let (envelope, summary) =
        crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

    assert_eq!(envelope["tool"], "shell.exec");
    assert_eq!(envelope["payload_truncated"], true);
    assert_eq!(summary["command"], "cargo");
    assert_eq!(summary["exit_code"], 0);
    assert!(summary.get("stdout_preview").is_some());
    assert!(summary.get("stdout_chars").is_some());
    assert_eq!(summary["stdout_truncated"], true);
    assert!(summary.get("stderr_preview").is_some());
    assert!(summary.get("stderr_chars").is_some());
    assert_eq!(summary["stderr_truncated"], true);
    assert!(
        summary["stdout_preview"]
            .as_str()
            .expect("stdout preview should exist")
            .contains("stdout line 0"),
        "expected compact stdout preview, got: {summary:?}"
    );
    assert!(
        summary["stderr_preview"]
            .as_str()
            .expect("stderr preview should exist")
            .contains("stderr line 0"),
        "expected compact stderr preview, got: {summary:?}"
    );
}

#[test]
fn build_turn_reply_followup_messages_compacts_tool_search_payload_summary() {
    let payload_summary = serde_json::json!({
        "adapter": "core-tools",
        "tool_name": "tool.search",
        "query": "read repo file",
        "returned": 2,
        "results": [
            {
                "tool_id": "file.read",
                "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                "argument_hint": "path:string,offset?:integer,limit?:integer",
                "required_fields": ["path"],
                "required_field_groups": [["path"]],
                "tags": ["core", "file", "read"],
                "why": ["summary matches query", "tag matches read"],
                "lease": "lease-file"
            },
            {
                "tool_id": "shell.exec",
                "summary": "Execute a shell command in the workspace.",
                "argument_hint": "command:string,args?:string[]",
                "required_fields": ["command"],
                "required_field_groups": [["command"]],
                "tags": ["core", "shell", "exec"],
                "why": ["summary matches query", "tag matches exec"],
                "lease": "lease-shell"
            }
        ]
    });
    let payload_summary_str = payload_summary.to_string();
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_chars": 2_048,
            "payload_summary": payload_summary_str,
            "payload_truncated": false
        })
    );

    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "find the right tool",
    );

    let (envelope, summary) =
        crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);
    let summary_str = envelope["payload_summary"]
        .as_str()
        .expect("payload summary should stay encoded json");
    let results = summary["results"]
        .as_array()
        .expect("results should be an array");
    let first = &results[0];

    assert_eq!(envelope["tool"], "tool.search");
    assert_eq!(envelope["payload_truncated"], false);
    assert_ne!(summary_str, payload_summary.to_string());
    assert_eq!(summary["query"], "read repo file");
    assert!(summary.get("adapter").is_none());
    assert!(summary.get("tool_name").is_none());
    assert_eq!(summary["returned"], 2);
    assert_eq!(results.len(), 2);
    assert_eq!(first["tool_id"], "file.read");
    assert_eq!(first["lease"], "lease-file");
    for entry in results {
        assert!(entry.get("tool_id").and_then(Value::as_str).is_some());
        assert!(entry.get("summary").and_then(Value::as_str).is_some());
        assert!(entry.get("argument_hint").and_then(Value::as_str).is_some());
        assert!(
            entry
                .get("required_fields")
                .and_then(Value::as_array)
                .is_some()
        );
        assert!(
            entry
                .get("required_field_groups")
                .and_then(Value::as_array)
                .is_some()
        );
        assert!(entry.get("lease").and_then(Value::as_str).is_some());
        assert!(entry.get("tags").is_none());
        assert!(entry.get("why").is_none());
    }
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn pending_approval_control_turn_bootstraps_once_and_emits_terminal_phases_once() {
    let runtime = ApprovalControlRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = LoongClawConfig::default();
    let memory_config = sqlite_memory_config("approval-control-bootstrap");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(&repo, "root-session", "apr-deny-1", "delegate_async", "app");

    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("root-session");
    let kernel_ctx = crate::context::bootstrap_test_kernel_context("approval-control-observer", 60)
        .expect("kernel context");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "esc",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
            Some(observer_handle),
        )
        .await
        .expect("approval control turn should succeed");

    assert_eq!(reply, "approval handled");

    let bootstrap_calls = runtime
        .bootstrap_calls
        .lock()
        .expect("bootstrap call lock should not be poisoned");
    assert_eq!(*bootstrap_calls, 1);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();

    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::ContextReady,
            ConversationTurnPhase::RunningTools,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn pending_approval_control_turn_does_not_persist_session_mode_when_resolution_fails() {
    let coordinator = ConversationTurnCoordinator::new();
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongClawConfig::default();
    let memory_config = sqlite_memory_config("approval-control-session-mode");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(
        &repo,
        "root-session",
        "apr-auto-failure",
        "delegate_async",
        "app",
    );

    let db_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .to_path_buf();
    let conn = rusqlite::Connection::open(db_path).expect("open sqlite connection");
    conn.execute_batch(
        "
            CREATE TRIGGER fail_pending_approval_update
            BEFORE UPDATE ON approval_requests
            BEGIN
                SELECT RAISE(FAIL, 'forced approval request update failure');
            END;
            ",
    )
    .expect("create approval failure trigger");

    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("root-session");
    let result = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "auto",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            None,
        )
        .await;
    assert!(
        result.is_ok(),
        "approval control turn should stay user-visible"
    );

    let stored = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent");
    assert!(
        stored.is_none(),
        "session mode should not persist on failure"
    );

    let approval_request = repo
        .load_approval_request("apr-auto-failure")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(approval_request.status, ApprovalRequestStatus::Pending);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_request_resolve_persists_session_mode_on_success() {
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongClawConfig::default();
    let memory_config = sqlite_memory_config("approval-control-session-mode-success");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(
        &repo,
        "root-session",
        "apr-auto-success",
        "sessions_list",
        "app",
    );
    let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::direct(),
    );
    let outcome = crate::tools::approval::execute_approval_tool_with_runtime_support(
        loongclaw_contracts::ToolCoreRequest {
            tool_name: "approval_request_resolve".to_owned(),
            payload: json!({
                "approval_request_id": "apr-auto-success",
                "decision": "approve_once",
                "session_consent_mode": "auto",
            }),
        },
        "root-session",
        &memory_config,
        &ToolConfig::default(),
        Some(&approval_runtime),
    )
    .await
    .expect("approval request resolve should succeed");
    assert_eq!(outcome.payload["approval_request"]["status"], "approved");

    let stored = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent")
        .expect("session tool consent row");
    assert_eq!(stored.mode, ToolConsentMode::Auto);
    assert_eq!(
        stored.updated_by_session_id.as_deref(),
        Some("root-session")
    );

    let approval_request = repo
        .load_approval_request("apr-auto-success")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(
        approval_request.decision,
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(approval_request.status, ApprovalRequestStatus::Approved);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_request_resolve_retries_missing_session_mode_after_approval() {
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongClawConfig::default();
    let memory_config = sqlite_memory_config("approval-control-session-mode-retry");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(
        &repo,
        "root-session",
        "apr-auto-retry",
        "sessions_list",
        "app",
    );
    repo.transition_approval_request_if_current(
        "apr-auto-retry",
        TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Pending,
            next_status: ApprovalRequestStatus::Approved,
            decision: Some(ApprovalDecision::ApproveOnce),
            resolved_by_session_id: Some("root-session".to_owned()),
            executed_at: None,
            last_error: None,
        },
    )
    .expect("transition approval request")
    .expect("approval request should be pending");

    let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::direct(),
    );
    let outcome = crate::tools::approval::execute_approval_tool_with_runtime_support(
        loongclaw_contracts::ToolCoreRequest {
            tool_name: "approval_request_resolve".to_owned(),
            payload: json!({
                "approval_request_id": "apr-auto-retry",
                "decision": "approve_once",
                "session_consent_mode": "auto",
            }),
        },
        "root-session",
        &memory_config,
        &ToolConfig::default(),
        Some(&approval_runtime),
    )
    .await
    .expect("approval request retry should succeed");

    assert_eq!(outcome.payload["approval_request"]["status"], "approved");

    let stored = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent")
        .expect("session tool consent row");
    assert_eq!(stored.mode, ToolConsentMode::Auto);
    assert_eq!(
        stored.updated_by_session_id.as_deref(),
        Some("root-session")
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn core_approval_replay_skips_app_session_context_loading() {
    let runtime = CoreReplayRuntime;
    let mut config = LoongClawConfig::default();
    let memory_config = sqlite_memory_config("approval-core-replay");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-core-replay".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-core-replay".to_owned(),
        tool_call_id: "call-core-replay".to_owned(),
        tool_name: "provider.switch".to_owned(),
        approval_key: "tool:provider.switch".to_owned(),
        request_payload_json: json!({
            "session_id": "root-session",
            "turn_id": "turn-core-replay",
            "tool_call_id": "call-core-replay",
            "tool_name": "provider.switch",
            "args_json": {
                "selector": "openai"
            },
            "source": "test",
            "execution_kind": "core",
        }),
        governance_snapshot_json: json!({
            "rule_id": "session_tool_consent_auto_blocked",
        }),
    })
    .expect("seed core approval request");
    let approval_request = repo
        .load_approval_request("apr-core-replay")
        .expect("load approval request")
        .expect("approval request row");
    let kernel_ctx =
        bootstrap_test_kernel_context("approval-core-replay", 60).expect("kernel context");
    let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
    );

    let error = approval_runtime
        .replay_approved_request(&approval_request)
        .await
        .expect_err("provider.switch should fail at core execution");
    assert!(
        error.contains("provider.switch requires a resolved runtime config path"),
        "expected core execution failure, got: {error}"
    );
}

#[test]
fn provider_turn_session_state_appends_user_input_and_keeps_estimate() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext {
            messages: vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            artifacts: vec![],
            estimated_tokens: Some(42),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        },
        "hello world",
        None,
    );

    assert_eq!(session.estimated_tokens, Some(42));
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[1]["role"], "user");
    assert_eq!(session.messages[1]["content"], "hello world");
}

#[test]
fn provider_turn_session_state_after_turn_messages_appends_reply() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "hello world",
        None,
    );

    let messages = session.after_turn_messages("done");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["content"], "done");
}

#[test]
fn provider_turn_reply_tail_phase_captures_reply_and_after_turn_context() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext {
            messages: vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            artifacts: vec![],
            estimated_tokens: Some(42),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        },
        "hello world",
        None,
    );

    let phase = ProviderTurnReplyTailPhase::from_session(&session, "done");

    assert_eq!(phase.reply(), "done");
    assert_eq!(phase.estimated_tokens(), Some(42));
    assert_eq!(phase.after_turn_messages().len(), 3);
    assert_eq!(phase.after_turn_messages()[2]["role"], "assistant");
    assert_eq!(phase.after_turn_messages()[2]["content"], "done");
}

#[test]
fn provider_turn_lane_plan_hybrid_disabled_forces_fast_lane_limits() {
    let mut config = LoongClawConfig::default();
    config.conversation.hybrid_lane_enabled = false;
    config.conversation.fast_lane_max_tool_steps_per_turn = 3;
    config.conversation.safe_lane_max_tool_steps_per_turn = 7;

    let plan = ProviderTurnLanePlan::from_user_input(&config, "deploy to production");

    assert_eq!(plan.decision.lane, ExecutionLane::Fast);
    assert_eq!(plan.max_tool_steps, 3);
    assert!(
        plan.decision
            .reasons
            .iter()
            .any(|reason| reason.contains("hybrid_lane_disabled"))
    );
}

#[test]
fn provider_turn_preparation_derives_lane_plan_and_raw_mode() {
    let mut config = LoongClawConfig::default();
    config.conversation.fast_lane_max_tool_steps_per_turn = 2;
    config.conversation.safe_lane_max_tool_steps_per_turn = 5;

    let preparation = ProviderTurnPreparation::from_assembled_context(
        &config,
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "deploy to production and show raw tool output",
        None,
    );

    assert_eq!(preparation.session.messages.len(), 2);
    assert_eq!(preparation.session.messages[1]["role"], "user");
    assert_eq!(
        preparation.session.messages[1]["content"],
        "deploy to production and show raw tool output"
    );
    assert!(preparation.raw_tool_output_requested);
    assert_eq!(preparation.lane_plan.decision.lane, ExecutionLane::Safe);
    assert_eq!(preparation.lane_plan.max_tool_steps, 5);
}

#[test]
fn provider_turn_lane_plan_safe_plan_path_requires_safe_lane_and_tool_intents() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_plan_execution_enabled = true;

    let safe_plan =
        ProviderTurnLanePlan::from_user_input(&config, "deploy to production and rotate the token");
    let tool_turn = ProviderTurn {
        assistant_text: "preface".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: json!({"command": "echo hi"}),
            source: "provider_tool_call".to_owned(),
            session_id: "session-safe".to_owned(),
            turn_id: "turn-safe".to_owned(),
            tool_call_id: "call-safe".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    assert_eq!(safe_plan.decision.lane, ExecutionLane::Safe);
    assert!(safe_plan.should_use_safe_lane_plan_path(&config, &tool_turn));
    assert!(!safe_plan.should_use_safe_lane_plan_path(
        &config,
        &ProviderTurn {
            tool_intents: Vec::new(),
            ..tool_turn.clone()
        }
    ));

    let fast_plan = ProviderTurnLanePlan::from_user_input(&config, "say hello");
    assert_eq!(fast_plan.decision.lane, ExecutionLane::Fast);
    assert!(!fast_plan.should_use_safe_lane_plan_path(&config, &tool_turn));
}

#[test]
fn provider_turn_continue_phase_checkpoint_captures_continue_branch_kernel_shape() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_max_tool_steps_per_turn = 5;
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &config,
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "deploy to production",
        None,
    );
    let phase = ProviderTurnContinuePhase::new(
        2,
        ProviderTurnLaneExecution {
            lane: ExecutionLane::Safe,
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            tool_request_summary: None,
            requires_provider_turn_followup: false,
            raw_tool_output_requested: false,
            turn_result: TurnResult::ToolError(TurnFailure::retryable(
                "safe_lane_plan_node_retryable_error",
                "transient",
            )),
            safe_lane_terminal_route: Some(SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                source: SafeLaneFailureRouteSource::SessionGovernor,
            }),
            tool_events: Vec::new(),
        },
        None,
        config,
        None,
    );

    let checkpoint = phase.checkpoint(&preparation, "deploy to production", "preface\ntransient");

    assert_eq!(
        checkpoint.request,
        TurnCheckpointRequest::Continue { tool_intents: 2 }
    );
    assert_eq!(
        checkpoint
            .lane
            .as_ref()
            .expect("lane snapshot should be present")
            .result_kind,
        TurnCheckpointResultKind::ToolError
    );
    assert_eq!(
        checkpoint
            .lane
            .as_ref()
            .and_then(|lane| lane.safe_lane_terminal_route)
            .expect("safe-lane route should be present")
            .source,
        SafeLaneFailureRouteSource::SessionGovernor
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .expect("reply checkpoint should be present")
            .decision,
        ReplyResolutionMode::CompletionPass
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .and_then(|reply| reply.followup_kind),
        Some(ToolDrivenFollowupKind::ToolFailure)
    );
    assert_eq!(
        checkpoint.finalization,
        TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::Success,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    );
    assert_eq!(
        checkpoint
            .identity
            .as_ref()
            .expect("identity should be present")
            .assistant_reply_chars,
        "preface\ntransient".chars().count()
    );
}

#[test]
fn scope_provider_turn_tool_intents_overrides_existing_provider_ids_with_runtime_scope() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "tool.search".to_owned(),
                args_json: json!({"query": "read file"}),
                source: "provider_tool_call".to_owned(),
                session_id: String::new(),
                turn_id: String::new(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "tool.invoke".to_owned(),
                args_json: json!({"tool_id": "file.read", "lease": "stub", "arguments": {"path": "README.md"}}),
                source: "provider_tool_call".to_owned(),
                session_id: "already-session".to_owned(),
                turn_id: "already-turn".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let scoped = scope_provider_turn_tool_intents(turn, "session-a", "turn-a");

    // Provider-originated intents always get runtime scope overridden.
    assert_eq!(scoped.tool_intents[0].session_id, "session-a");
    assert_eq!(scoped.tool_intents[0].turn_id, "turn-a");
    assert_eq!(scoped.tool_intents[1].session_id, "session-a");
    assert_eq!(scoped.tool_intents[1].turn_id, "turn-a");
}

#[test]
fn scope_non_provider_turn_tool_intents_preserve_existing_ids() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "tool.search".to_owned(),
                args_json: json!({"query": "read file"}),
                source: "local_followup".to_owned(),
                session_id: "existing-session".to_owned(),
                turn_id: "existing-turn".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "tool.invoke".to_owned(),
                args_json: json!({"tool_id": "file.read", "lease": "stub", "arguments": {"path": "README.md"}}),
                source: "local_followup".to_owned(),
                session_id: String::new(),
                turn_id: String::new(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let scoped = scope_provider_turn_tool_intents(turn, "session-a", "turn-a");

    assert_eq!(scoped.tool_intents[0].session_id, "existing-session");
    assert_eq!(scoped.tool_intents[0].turn_id, "existing-turn");
    assert_eq!(scoped.tool_intents[1].session_id, "session-a");
    assert_eq!(scoped.tool_intents[1].turn_id, "turn-a");
}

#[test]
fn reload_followup_provider_config_reads_provider_switch_wrapped_by_tool_invoke() {
    use std::fs;

    let root = std::env::temp_dir().join(format!(
        "loongclaw-provider-switch-followup-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");

    let mut expected = LoongClawConfig::default();
    let mut openai =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    expected.set_active_provider_profile(
        "openai-gpt-5",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai.clone(),
        },
    );
    expected.provider = openai;
    expected.active_provider = Some("openai-gpt-5".to_owned());
    fs::write(
        &config_path,
        crate::config::render(&expected).expect("render config"),
    )
    .expect("write config");

    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".to_owned(),
            args_json: json!({
                "tool_id": "provider.switch",
                "lease": "ignored",
                "arguments": {
                    "selector": "openai",
                    "config_path": config_path.to_string_lossy()
                }
            }),
            source: "provider_tool_call".to_owned(),
            session_id: "session-a".to_owned(),
            turn_id: "turn-a".to_owned(),
            tool_call_id: "call-1".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    let reloaded = ConversationTurnCoordinator::reload_followup_provider_config_after_tool_turn(
        &LoongClawConfig::default(),
        &turn,
    );

    assert_eq!(reloaded.active_provider_id(), Some("openai-gpt-5"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn provider_turn_continue_phase_checkpoint_keeps_direct_reply_without_followup() {
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongClawConfig::default(),
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );
    let phase = ProviderTurnContinuePhase::new(
        0,
        ProviderTurnLaneExecution {
            lane: ExecutionLane::Fast,
            assistant_preface: "preface".to_owned(),
            had_tool_intents: false,
            tool_request_summary: None,
            requires_provider_turn_followup: false,
            raw_tool_output_requested: false,
            turn_result: TurnResult::FinalText("hello there".to_owned()),
            safe_lane_terminal_route: None,
            tool_events: Vec::new(),
        },
        None,
        LoongClawConfig::default(),
        None,
    );

    let checkpoint = phase.checkpoint(&preparation, "say hello", "hello there");

    assert_eq!(
        checkpoint.request,
        TurnCheckpointRequest::Continue { tool_intents: 0 }
    );
    assert_eq!(
        checkpoint
            .lane
            .as_ref()
            .expect("lane snapshot should be present")
            .result_kind,
        TurnCheckpointResultKind::FinalText
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .expect("reply checkpoint should be present")
            .decision,
        ReplyResolutionMode::Direct
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .and_then(|reply| reply.followup_kind),
        None
    );
    assert_eq!(
        checkpoint
            .identity
            .as_ref()
            .expect("identity should be present")
            .assistant_reply_chars,
        "hello there".chars().count()
    );
}

#[test]
fn resolved_provider_turn_checkpoint_preserves_safe_lane_route_provenance() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_max_tool_steps_per_turn = 5;

    let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
        reply: "preface\nsafe lane terminal".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: Some(TurnCheckpointIdentity::from_turn(
                "deploy to production",
                "preface\nsafe lane terminal",
            )),
            preparation: ProviderTurnPreparation::from_assembled_context(
                &config,
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "deploy to production",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::Continue { tool_intents: 1 },
            lane: Some(TurnLaneExecutionSnapshot {
                lane: ExecutionLane::Safe,
                had_tool_intents: true,
                tool_request_summary: None,
                raw_tool_output_requested: false,
                result_kind: TurnCheckpointResultKind::ToolError,
                safe_lane_terminal_route: Some(SafeLaneFailureRoute {
                    decision: SafeLaneFailureRouteDecision::Terminal,
                    reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                    source: SafeLaneFailureRouteSource::SessionGovernor,
                }),
            }),
            reply: Some(TurnReplyCheckpoint {
                decision: ReplyResolutionMode::CompletionPass,
                followup_kind: Some(ToolDrivenFollowupKind::ToolFailure),
            }),
            finalization: TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::Success,
                runs_after_turn: true,
                attempts_context_compaction: true,
            },
        },
    });
    let snapshot = resolved.checkpoint();

    assert_eq!(snapshot.preparation.lane, ExecutionLane::Safe);
    assert_eq!(snapshot.preparation.context_message_count, 2);
    assert_eq!(
        snapshot.preparation.context_fingerprint_sha256,
        checkpoint_context_fingerprint_sha256(&[
            serde_json::json!({
                "role": "system",
                "content": "sys"
            }),
            serde_json::json!({
                "role": "user",
                "content": "deploy to production"
            }),
        ])
    );
    assert_eq!(
        snapshot.request,
        TurnCheckpointRequest::Continue { tool_intents: 1 }
    );
    assert_eq!(
        snapshot.lane.as_ref().expect("lane snapshot").result_kind,
        TurnCheckpointResultKind::ToolError
    );
    assert_eq!(
        snapshot
            .lane
            .as_ref()
            .and_then(|lane| lane.safe_lane_terminal_route)
            .expect("safe-lane route")
            .source,
        SafeLaneFailureRouteSource::SessionGovernor
    );
    assert_eq!(
        snapshot.reply.as_ref().expect("reply checkpoint").decision,
        ReplyResolutionMode::CompletionPass
    );
    assert_eq!(
        snapshot
            .reply
            .as_ref()
            .and_then(|reply| reply.followup_kind),
        Some(ToolDrivenFollowupKind::ToolFailure)
    );
    assert_eq!(
        snapshot.finalization,
        TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::Success,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    );
    assert_eq!(
        snapshot
            .identity
            .as_ref()
            .expect("identity should be present")
            .user_input_chars,
        "deploy to production".chars().count()
    );
    assert_eq!(resolved.reply_text(), Some("preface\nsafe lane terminal"));
}

#[test]
fn resolved_provider_turn_checkpoint_keeps_inline_provider_error_terminal_shape() {
    let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
        reply: "provider unavailable".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: Some(TurnCheckpointIdentity::from_turn(
                "say hello",
                "provider unavailable",
            )),
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongClawConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::FinalizeInlineProviderError,
            lane: None,
            reply: None,
            finalization: TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::InlineProviderError,
                runs_after_turn: true,
                attempts_context_compaction: true,
            },
        },
    });
    let snapshot = resolved.checkpoint();

    assert_eq!(
        snapshot.request,
        TurnCheckpointRequest::FinalizeInlineProviderError
    );
    assert!(snapshot.lane.is_none());
    assert!(snapshot.reply.is_none());
    assert!(snapshot.identity.is_some());
    assert_eq!(
        snapshot.finalization,
        TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::InlineProviderError,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    );
    assert_eq!(resolved.reply_text(), Some("provider unavailable"));
}

#[test]
fn resolved_provider_turn_checkpoint_marks_return_error_finalization() {
    let resolved = ResolvedProviderTurn::ReturnError(ResolvedProviderError {
        error: "provider unavailable".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: None,
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongClawConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::ReturnError,
            lane: None,
            reply: None,
            finalization: TurnFinalizationCheckpoint::ReturnError,
        },
    });
    let snapshot = resolved.checkpoint();

    assert_eq!(snapshot.request, TurnCheckpointRequest::ReturnError);
    assert!(snapshot.identity.is_none());
    assert!(snapshot.lane.is_none());
    assert!(snapshot.reply.is_none());
    assert_eq!(
        snapshot.finalization,
        TurnFinalizationCheckpoint::ReturnError
    );
    assert_eq!(resolved.reply_text(), None);
}

#[test]
fn resolved_provider_turn_terminal_phase_builds_reply_tail_and_checkpoint() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext {
            messages: vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            artifacts: vec![],
            estimated_tokens: Some(42),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        },
        "say hello",
        None,
    );
    let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
        reply: "done".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: Some(TurnCheckpointIdentity::from_turn("say hello", "done")),
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongClawConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::Continue { tool_intents: 0 },
            lane: Some(TurnLaneExecutionSnapshot {
                lane: ExecutionLane::Fast,
                had_tool_intents: false,
                tool_request_summary: None,
                raw_tool_output_requested: false,
                result_kind: TurnCheckpointResultKind::FinalText,
                safe_lane_terminal_route: None,
            }),
            reply: Some(TurnReplyCheckpoint {
                decision: ReplyResolutionMode::Direct,
                followup_kind: None,
            }),
            finalization: TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
        },
    });

    let phase = resolved.terminal_phase(&session);

    match phase {
        ProviderTurnTerminalPhase::PersistReply(phase) => {
            assert_eq!(
                phase.checkpoint.request,
                TurnCheckpointRequest::Continue { tool_intents: 0 }
            );
            assert_eq!(phase.tail_phase.reply(), "done");
            assert_eq!(phase.tail_phase.estimated_tokens(), Some(42));
            assert_eq!(phase.tail_phase.after_turn_messages().len(), 3);
            assert_eq!(phase.tail_phase.after_turn_messages()[2]["content"], "done");
        }
        ProviderTurnTerminalPhase::ReturnError(_) => {
            panic!("persist reply should build persist terminal phase")
        }
    }
}

#[test]
fn resolved_provider_turn_terminal_phase_preserves_return_error_checkpoint() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );
    let resolved = ResolvedProviderTurn::ReturnError(ResolvedProviderError {
        error: "provider unavailable".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: None,
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongClawConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::ReturnError,
            lane: None,
            reply: None,
            finalization: TurnFinalizationCheckpoint::ReturnError,
        },
    });

    let phase = resolved.terminal_phase(&session);

    match phase {
        ProviderTurnTerminalPhase::ReturnError(phase) => {
            assert_eq!(phase.checkpoint.request, TurnCheckpointRequest::ReturnError);
            assert_eq!(phase.error, "provider unavailable");
        }
        ProviderTurnTerminalPhase::PersistReply(_) => {
            panic!("return error should build return-error terminal phase")
        }
    }
}

#[test]
fn provider_turn_request_terminal_phase_builds_inline_provider_error_reply() {
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongClawConfig::default(),
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );

    let resolved = ProviderTurnRequestTerminalPhase::persist_inline_provider_error(
        "provider unavailable".to_owned(),
    )
    .resolve(&preparation, "say hello");

    match resolved {
        ResolvedProviderTurn::PersistReply(reply) => {
            assert_eq!(reply.reply, "provider unavailable");
            assert_eq!(
                reply.checkpoint.request,
                TurnCheckpointRequest::FinalizeInlineProviderError
            );
            assert!(reply.checkpoint.lane.is_none());
            assert!(reply.checkpoint.reply.is_none());
            assert_eq!(
                reply.checkpoint.finalization,
                TurnFinalizationCheckpoint::persist_reply(
                    ReplyPersistenceMode::InlineProviderError,
                )
            );
            assert!(reply.checkpoint.identity.is_some());
        }
        ResolvedProviderTurn::ReturnError(_) => {
            panic!("inline provider error should resolve to persisted reply")
        }
    }
}

#[test]
fn provider_turn_request_terminal_phase_builds_return_error_without_reply_identity() {
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongClawConfig::default(),
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );

    let resolved =
        ProviderTurnRequestTerminalPhase::return_error("provider unavailable".to_owned())
            .resolve(&preparation, "say hello");

    match resolved {
        ResolvedProviderTurn::ReturnError(error) => {
            assert_eq!(error.error, "provider unavailable");
            assert_eq!(error.checkpoint.request, TurnCheckpointRequest::ReturnError);
            assert!(error.checkpoint.identity.is_none());
            assert!(error.checkpoint.lane.is_none());
            assert!(error.checkpoint.reply.is_none());
            assert_eq!(
                error.checkpoint.finalization,
                TurnFinalizationCheckpoint::ReturnError
            );
        }
        ResolvedProviderTurn::PersistReply(_) => {
            panic!("propagated provider error should resolve to return-error outcome")
        }
    }
}

#[test]
fn safe_lane_replan_budget_allows_one_retry_then_exhausts() {
    let initial = SafeLaneReplanBudget::new(1);

    assert_eq!(
        initial.continuation_decision(),
        SafeLaneContinuationBudgetDecision::Continue
    );
    assert_eq!(initial.current_round(), 0);

    let exhausted = initial.after_replan();
    assert_eq!(
        exhausted.continuation_decision(),
        SafeLaneContinuationBudgetDecision::Terminal {
            reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
        }
    );
    assert_eq!(exhausted.current_round(), 1);
}

#[test]
fn escalating_attempt_budget_caps_growth_at_maximum() {
    let budget = EscalatingAttemptBudget::new(2, 4);

    assert_eq!(budget.current_limit(), 2);
    assert_eq!(budget.after_retry().current_limit(), 3);
    assert_eq!(budget.after_retry().after_retry().current_limit(), 4);
    assert_eq!(
        budget
            .after_retry()
            .after_retry()
            .after_retry()
            .current_limit(),
        4
    );
}

#[test]
fn decide_provider_request_action_continues_on_success() {
    let decision = decide_provider_turn_request_action(
        Ok(ProviderTurn {
            assistant_text: "preface".to_owned(),
            tool_intents: Vec::new(),
            raw_meta: Value::Null,
        }),
        ProviderErrorMode::Propagate,
    );

    if let ProviderTurnRequestAction::Continue { turn } = decision {
        assert_eq!(turn.assistant_text, "preface");
        assert!(turn.tool_intents.is_empty());
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_provider_request_action_inlines_synthetic_reply_when_requested() {
    let decision = decide_provider_turn_request_action(
        Err("provider unavailable".to_owned()),
        ProviderErrorMode::InlineMessage,
    );

    if let ProviderTurnRequestAction::FinalizeInlineProviderError { reply } = decision {
        assert!(reply.contains("provider unavailable"));
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_provider_request_action_returns_error_in_propagate_mode() {
    let decision = decide_provider_turn_request_action(
        Err("provider unavailable".to_owned()),
        ProviderErrorMode::Propagate,
    );

    if let ProviderTurnRequestAction::ReturnError { error } = decision {
        assert_eq!(error, "provider unavailable");
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn safe_lane_route_retryable_failure_replans_with_remaining_budget() {
    let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(1));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Replan);
    assert_eq!(route.reason, SafeLaneFailureRouteReason::RetryableFailure);
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
    assert_eq!(route.reason.as_str(), "retryable_failure");
}

#[test]
fn safe_lane_route_retryable_failure_becomes_terminal_after_budget_exhaustion() {
    let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
    let route =
        SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(1).after_replan());

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::RoundBudgetExhausted
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
    assert!(route.is_base_round_budget_terminal());
}

#[test]
fn safe_lane_route_policy_denied_failure_is_terminal() {
    let failure = TurnFailure::policy_denied("safe_lane_plan_node_policy_denied", "denied");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(route.reason, SafeLaneFailureRouteReason::PolicyDenied);
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
}

#[test]
fn safe_lane_route_non_retryable_failure_is_terminal() {
    let failure = TurnFailure::non_retryable("safe_lane_plan_node_non_retryable_error", "bad");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::NonRetryableFailure
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
}

#[test]
fn turn_failure_from_plan_failure_node_error_mapping_is_stable() {
    let cases = [
        (
            PlanNodeErrorKind::ApprovalRequired,
            TurnFailureKind::PolicyDenied,
            "safe_lane_plan_node_policy_denied",
            false,
        ),
        (
            PlanNodeErrorKind::PolicyDenied,
            TurnFailureKind::PolicyDenied,
            "safe_lane_plan_node_policy_denied",
            false,
        ),
        (
            PlanNodeErrorKind::Retryable,
            TurnFailureKind::Retryable,
            "safe_lane_plan_node_retryable_error",
            true,
        ),
        (
            PlanNodeErrorKind::NonRetryable,
            TurnFailureKind::NonRetryable,
            "safe_lane_plan_node_non_retryable_error",
            false,
        ),
    ];

    for (node_kind, expected_kind, expected_code, expected_retryable) in cases {
        let failure = PlanRunFailure::NodeFailed {
            node_id: "tool-1".to_owned(),
            attempts_used: 1,
            last_error_kind: node_kind,
            last_error: "boom".to_owned(),
        };
        let mapped = turn_failure_from_plan_failure(&failure);
        assert_eq!(mapped.kind, expected_kind, "node_kind={node_kind:?}");
        assert_eq!(mapped.code, expected_code, "node_kind={node_kind:?}");
        assert_eq!(
            mapped.retryable, expected_retryable,
            "node_kind={node_kind:?}"
        );
    }
}

#[test]
fn turn_failure_from_plan_failure_static_failure_mapping_is_stable() {
    let failures = [
        PlanRunFailure::ValidationFailed("invalid".to_owned()),
        PlanRunFailure::TopologyResolutionFailed,
        PlanRunFailure::BudgetExceeded {
            attempts_used: 5,
            limit: 4,
        },
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms: 1200,
            limit_ms: 1000,
        },
    ];

    for failure in failures {
        let mapped = turn_failure_from_plan_failure(&failure);
        assert_eq!(mapped.kind, TurnFailureKind::NonRetryable);
        assert!(!mapped.retryable);
        assert!(
            mapped.code.starts_with("safe_lane_plan_"),
            "unexpected code: {}",
            mapped.code
        );
    }
}

#[test]
fn safe_lane_event_sampling_keeps_critical_events() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 3;

    let emitted = should_emit_safe_lane_event(
        &config,
        "final_status",
        &json!({
            "round": 1
        }),
    );
    assert!(emitted, "critical final_status event must always emit");
}

#[test]
fn safe_lane_event_sampling_skips_non_critical_rounds() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 2;
    config.conversation.safe_lane_event_adaptive_sampling = false;

    let emit_round_0 = should_emit_safe_lane_event(
        &config,
        "plan_round_started",
        &json!({
            "round": 0
        }),
    );
    let emit_round_1 = should_emit_safe_lane_event(
        &config,
        "plan_round_started",
        &json!({
            "round": 1
        }),
    );

    assert!(emit_round_0, "round 0 should pass sampling gate");
    assert!(!emit_round_1, "round 1 should be sampled out");
}

#[test]
fn safe_lane_event_sampling_adaptive_mode_keeps_failure_pressure_events() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 4;
    config.conversation.safe_lane_event_adaptive_sampling = true;
    config
        .conversation
        .safe_lane_event_adaptive_failure_threshold = 1;

    let emitted = should_emit_safe_lane_event(
        &config,
        "plan_round_completed",
        &json!({
            "round": 1,
            "failure_code": "safe_lane_plan_node_retryable_error",
            "route_decision": "replan",
            "metrics": {
                "rounds_started": 2,
                "rounds_succeeded": 0,
                "rounds_failed": 1,
                "verify_failures": 0,
                "replans_triggered": 1,
                "total_attempts_used": 2
            }
        }),
    );

    assert!(
        emitted,
        "adaptive failure-pressure sampling should force emit for troubleshooting"
    );
}

#[test]
fn safe_lane_event_sampling_adaptive_mode_can_be_disabled() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 4;
    config.conversation.safe_lane_event_adaptive_sampling = false;
    config
        .conversation
        .safe_lane_event_adaptive_failure_threshold = 1;

    let emitted = should_emit_safe_lane_event(
        &config,
        "plan_round_completed",
        &json!({
            "round": 1,
            "failure_code": "safe_lane_plan_node_retryable_error",
            "route_decision": "replan",
            "metrics": {
                "rounds_started": 2,
                "rounds_succeeded": 0,
                "rounds_failed": 1,
                "verify_failures": 0,
                "replans_triggered": 1,
                "total_attempts_used": 2
            }
        }),
    );

    assert!(
        !emitted,
        "with adaptive sampling disabled, round-based sampling should still drop this event"
    );
}

#[test]
fn safe_lane_failure_pressure_counts_truncated_tool_output_stats() {
    let payload = json!({
        "tool_output_stats": {
            "output_lines": 1,
            "result_lines": 1,
            "truncated_result_lines": 1,
            "any_truncated": true,
            "truncation_ratio_milli": 1000
        }
    });
    assert_eq!(safe_lane_failure_pressure(&payload), 1);
}

#[test]
fn safe_lane_tool_output_stats_detect_truncated_result_lines() {
    let outputs = vec![
        "[ok] {\"payload_truncated\":true}".to_owned(),
        "[ok] {\"payload_truncated\":false}\n[tool_result_truncated] removed_chars=2".to_owned(),
        "plain diagnostic line".to_owned(),
    ];

    let stats = summarize_safe_lane_tool_output_stats(outputs.as_slice());
    assert_eq!(stats.output_lines, 4);
    assert_eq!(stats.result_lines, 3);
    assert_eq!(stats.truncated_result_lines, 2);
    assert_eq!(stats.truncation_ratio_milli(), 666);
    let encoded = stats.as_json();
    assert_eq!(encoded["any_truncated"], true);
    assert_eq!(encoded["truncation_ratio_milli"], 666);
}

#[test]
fn safe_lane_tool_output_stats_handles_mixed_multiline_blocks() {
    let outputs = vec![
            "\n[ok] {\"payload_truncated\":false}\nnot a result line\n[ok] {\"payload_truncated\":true}\n"
                .to_owned(),
            "[result] completed\n\n[ok] {\"payload_truncated\":false}".to_owned(),
        ];

    let stats = summarize_safe_lane_tool_output_stats(outputs.as_slice());
    assert_eq!(stats.output_lines, 5);
    assert_eq!(stats.result_lines, 4);
    assert_eq!(stats.truncated_result_lines, 1);
    assert_eq!(stats.truncation_ratio_milli(), 250);
    let encoded = stats.as_json();
    assert_eq!(encoded["any_truncated"], true);
    assert_eq!(encoded["truncation_ratio_milli"], 250);
}

#[test]
fn runtime_health_signal_marks_warn_on_truncation_pressure() {
    let mut config = LoongClawConfig::default();
    config
        .conversation
        .safe_lane_health_truncation_warn_threshold = 0.20;
    config
        .conversation
        .safe_lane_health_truncation_critical_threshold = 0.50;
    let metrics = SafeLaneExecutionMetrics {
        rounds_started: 2,
        tool_output_result_lines_total: 4,
        tool_output_truncated_result_lines_total: 1,
        ..SafeLaneExecutionMetrics::default()
    };

    let signal = derive_safe_lane_runtime_health_signal(&config, metrics, false, None);
    assert_eq!(signal.severity, "warn");
    assert!(
        signal
            .flags
            .iter()
            .any(|value| value.contains("truncation_pressure(0.250)"))
    );
}

#[test]
fn runtime_health_signal_marks_critical_on_terminal_instability() {
    let config = LoongClawConfig::default();
    let metrics = SafeLaneExecutionMetrics {
        rounds_started: 2,
        verify_failures: 1,
        replans_triggered: 1,
        tool_output_result_lines_total: 2,
        tool_output_truncated_result_lines_total: 1,
        ..SafeLaneExecutionMetrics::default()
    };

    let signal = derive_safe_lane_runtime_health_signal(
        &config,
        metrics,
        true,
        Some("safe_lane_plan_verify_failed_session_governor"),
    );
    assert_eq!(signal.severity, "critical");
    assert!(
        signal
            .flags
            .iter()
            .any(|value| value == "terminal_instability")
    );
}

#[test]
fn verify_anchor_policy_escalates_after_configured_failures() {
    let mut config = LoongClawConfig::default();
    config
        .conversation
        .safe_lane_verify_adaptive_anchor_escalation = true;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_after_failures = 2;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_min_matches = 1;

    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 0), 0);
    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 1), 0);
    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 2), 1);
    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 5), 1);
}

#[test]
fn verify_anchor_policy_escalation_can_be_disabled() {
    let mut config = LoongClawConfig::default();
    config
        .conversation
        .safe_lane_verify_adaptive_anchor_escalation = false;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_after_failures = 1;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_min_matches = 3;

    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 5), 0);
}

#[test]
fn backpressure_guard_blocks_replan_when_attempt_budget_exhausted() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 2;
    config.conversation.safe_lane_backpressure_max_replans = 10;

    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Replan,
        reason: SafeLaneFailureRouteReason::RetryableFailure,
        source: SafeLaneFailureRouteSource::BaseRouting,
    };
    let metrics = SafeLaneExecutionMetrics {
        total_attempts_used: 2,
        ..SafeLaneExecutionMetrics::default()
    };
    let guarded = route.with_backpressure_guard(safe_lane_backpressure_budget(&config), metrics);
    assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        guarded.reason,
        SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
    );
    assert_eq!(
        guarded.source,
        SafeLaneFailureRouteSource::BackpressureGuard
    );
}

#[test]
fn backpressure_guard_blocks_replan_when_replan_budget_exhausted() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 10;
    config.conversation.safe_lane_backpressure_max_replans = 1;

    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Replan,
        reason: SafeLaneFailureRouteReason::RetryableFailure,
        source: SafeLaneFailureRouteSource::BaseRouting,
    };
    let metrics = SafeLaneExecutionMetrics {
        replans_triggered: 1,
        ..SafeLaneExecutionMetrics::default()
    };
    let guarded = route.with_backpressure_guard(safe_lane_backpressure_budget(&config), metrics);
    assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        guarded.reason,
        SafeLaneFailureRouteReason::BackpressureReplansExhausted
    );
    assert_eq!(
        guarded.source,
        SafeLaneFailureRouteSource::BackpressureGuard
    );
}

fn governor_history_with_summary(summary: SafeLaneEventSummary) -> SafeLaneGovernorHistorySignals {
    SafeLaneGovernorHistorySignals {
        summary,
        ..SafeLaneGovernorHistorySignals::default()
    }
}

#[test]
fn safe_lane_backpressure_budget_detects_attempt_exhaustion() {
    let budget = SafeLaneBackpressureBudget::new(2, 10);
    let metrics = SafeLaneExecutionMetrics {
        total_attempts_used: 2,
        ..SafeLaneExecutionMetrics::default()
    };

    assert_eq!(
        budget.continuation_decision(metrics.total_attempts_used, metrics.replans_triggered),
        SafeLaneContinuationBudgetDecision::Terminal {
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
        }
    );
}

#[test]
fn decide_safe_lane_failure_route_applies_backpressure_after_retryable_base_route() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 2;
    config.conversation.safe_lane_backpressure_max_replans = 10;

    let route = decide_safe_lane_failure_route(
        &config,
        &TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient"),
        SafeLaneReplanBudget::new(3),
        SafeLaneExecutionMetrics {
            total_attempts_used: 2,
            ..SafeLaneExecutionMetrics::default()
        },
        &SafeLaneSessionGovernorDecision::default(),
    );

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BackpressureGuard);
}

#[test]
fn decide_safe_lane_failure_route_applies_session_governor_override_to_exhausted_budget() {
    let config = LoongClawConfig::default();
    let route = decide_safe_lane_failure_route(
        &config,
        &TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient"),
        SafeLaneReplanBudget::new(1).after_replan(),
        SafeLaneExecutionMetrics::default(),
        &SafeLaneSessionGovernorDecision {
            force_no_replan: true,
            ..SafeLaneSessionGovernorDecision::default()
        },
    );

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::SessionGovernorNoReplan
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::SessionGovernor);
}

#[test]
fn summarize_governor_history_signals_extracts_failure_samples() {
    let contents = [
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_backpressure_guard","route_reason":"backpressure_attempts_exhausted"}}"#,
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"succeeded"}}"#,
    ];

    let signals = summarize_governor_history_signals(contents.iter().copied());
    assert_eq!(signals.final_status_failed_samples, vec![true, false]);
    assert_eq!(signals.backpressure_failure_samples, vec![true, false]);
    assert_eq!(
        signals
            .summary
            .failure_code_counts
            .get("safe_lane_plan_backpressure_guard")
            .copied(),
        Some(1)
    );
}

#[test]
fn summarize_governor_history_signals_ignores_unknown_backpressure_like_strings() {
    let contents = [
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"unknown_backpressure_hint","route_reason":"backpressure_noise"}}"#,
    ];

    let signals = summarize_governor_history_signals(contents.iter().copied());
    assert_eq!(signals.final_status_failed_samples, vec![true]);
    assert_eq!(signals.backpressure_failure_samples, vec![false]);
}

#[test]
fn session_governor_engages_on_failed_final_status_threshold() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 2;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_force_no_replan = true;
    config
        .conversation
        .safe_lane_session_governor_force_node_max_attempts = 1;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 2);

    let history = governor_history_with_summary(summary);
    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.engaged);
    assert!(decision.failed_threshold_triggered);
    assert!(!decision.backpressure_threshold_triggered);
    assert!(decision.force_no_replan);
    assert_eq!(decision.forced_node_max_attempts, Some(1));
}

#[test]
fn session_governor_engages_on_backpressure_threshold() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 2;
    config
        .conversation
        .safe_lane_session_governor_force_node_max_attempts = 2;

    let mut summary = SafeLaneEventSummary::default();
    summary
        .failure_code_counts
        .insert("safe_lane_plan_backpressure_guard".to_owned(), 1);
    summary.failure_code_counts.insert(
        "safe_lane_plan_verify_failed_backpressure_guard".to_owned(),
        1,
    );

    let history = governor_history_with_summary(summary);
    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.engaged);
    assert!(!decision.failed_threshold_triggered);
    assert!(decision.backpressure_threshold_triggered);
    assert_eq!(decision.backpressure_failure_events, 2);
    assert_eq!(decision.forced_node_max_attempts, Some(2));
}

#[test]
fn session_governor_stays_disabled_when_thresholds_not_reached() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 3;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 2;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 1);
    summary
        .failure_code_counts
        .insert("safe_lane_plan_backpressure_guard".to_owned(), 1);

    let history = governor_history_with_summary(summary);
    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(!decision.engaged);
    assert!(!decision.force_no_replan);
    assert_eq!(decision.forced_node_max_attempts, None);
}

#[test]
fn session_governor_engages_on_trend_threshold_when_counts_are_low() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config.conversation.safe_lane_session_governor_trend_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_trend_min_samples = 4;
    config
        .conversation
        .safe_lane_session_governor_trend_ewma_alpha = 0.5;
    config
        .conversation
        .safe_lane_session_governor_trend_failure_ewma_threshold = 0.60;
    config
        .conversation
        .safe_lane_session_governor_trend_backpressure_ewma_threshold = 0.70;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 1);
    let history = SafeLaneGovernorHistorySignals {
        history_load_status: SafeLaneGovernorHistoryLoadStatus::Loaded,
        history_load_error: None,
        summary,
        final_status_failed_samples: vec![false, true, true, true],
        backpressure_failure_samples: vec![false, false, false, false],
    };

    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.engaged);
    assert!(!decision.failed_threshold_triggered);
    assert!(!decision.backpressure_threshold_triggered);
    assert!(decision.trend_threshold_triggered);
    assert!(
        decision
            .trend_failure_ewma
            .map(|value| value > 0.60)
            .unwrap_or(false)
    );
}

#[test]
fn session_governor_recovery_threshold_can_suppress_engagement() {
    let mut config = LoongClawConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 1;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config.conversation.safe_lane_session_governor_trend_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_trend_min_samples = 4;
    config
        .conversation
        .safe_lane_session_governor_trend_ewma_alpha = 0.5;
    config
        .conversation
        .safe_lane_session_governor_trend_failure_ewma_threshold = 0.70;
    config
        .conversation
        .safe_lane_session_governor_recovery_success_streak = 3;
    config
        .conversation
        .safe_lane_session_governor_recovery_max_failure_ewma = 0.30;
    config
        .conversation
        .safe_lane_session_governor_recovery_max_backpressure_ewma = 0.10;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 1);
    let history = SafeLaneGovernorHistorySignals {
        history_load_status: SafeLaneGovernorHistoryLoadStatus::Loaded,
        history_load_error: None,
        summary,
        final_status_failed_samples: vec![true, false, false, false, false],
        backpressure_failure_samples: vec![true, false, false, false, false],
    };

    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.failed_threshold_triggered);
    assert!(!decision.trend_threshold_triggered);
    assert!(decision.recovery_threshold_triggered);
    assert_eq!(decision.recovery_success_streak, 4);
    assert!(!decision.engaged);
}

#[test]
fn session_governor_route_override_marks_no_replan_terminal_reason() {
    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
        source: SafeLaneFailureRouteSource::BaseRouting,
    };
    let governor = SafeLaneSessionGovernorDecision {
        force_no_replan: true,
        ..SafeLaneSessionGovernorDecision::default()
    };
    let overridden = route.with_session_governor_override(&governor);
    assert_eq!(
        overridden.reason,
        SafeLaneFailureRouteReason::SessionGovernorNoReplan
    );
    assert_eq!(
        overridden.source,
        SafeLaneFailureRouteSource::SessionGovernor
    );
}

#[test]
fn terminal_verify_failure_uses_backpressure_error_code() {
    let failure = terminal_turn_failure_from_verify_failure(
        "retryable verify failure",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        },
    );
    assert_eq!(
        failure.code,
        "safe_lane_plan_verify_failed_backpressure_guard"
    );
    assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn safe_lane_terminal_verify_failure_code_prefers_budget_exhaustion_for_retryable_base_route() {
    let code = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
        source: SafeLaneFailureRouteSource::BaseRouting,
    }
    .terminal_verify_failure_code(true);
    assert_eq!(code, SafeLaneFailureCode::VerifyFailedBudgetExhausted);
}

#[test]
fn safe_lane_route_verify_summary_label_marks_backpressure_guard() {
    let label = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
        source: SafeLaneFailureRouteSource::BackpressureGuard,
    }
    .verify_terminal_summary_label();
    assert_eq!(label, "verify_failed_backpressure_guard");
}

#[test]
fn safe_lane_route_profile_methods_encode_decision_and_source_labels() {
    let route = SafeLaneFailureRoute::replan(SafeLaneFailureRouteReason::RetryableFailure);
    assert!(route.should_replan());
    assert_eq!(route.decision_label(), "replan");
    assert_eq!(route.source_label(), "base_routing");

    let terminal = SafeLaneFailureRoute::terminal_with_source(
        SafeLaneFailureRouteReason::SessionGovernorNoReplan,
        SafeLaneFailureRouteSource::SessionGovernor,
    );
    assert!(!terminal.should_replan());
    assert_eq!(terminal.decision_label(), "terminal");
    assert_eq!(terminal.source_label(), "session_governor");
}

#[test]
fn safe_lane_route_backpressure_transition_is_localized_on_route() {
    let route = SafeLaneFailureRoute::replan(SafeLaneFailureRouteReason::RetryableFailure)
        .with_backpressure_guard(
            Some(SafeLaneBackpressureBudget::new(2, 10)),
            SafeLaneExecutionMetrics {
                total_attempts_used: 2,
                ..SafeLaneExecutionMetrics::default()
            },
        );
    assert!(!route.should_replan());
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BackpressureGuard);
}

#[test]
fn terminal_verify_failure_uses_budget_exhaustion_error_code() {
    let failure = terminal_turn_failure_from_verify_failure(
        "retryable verify failure",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            source: SafeLaneFailureRouteSource::BaseRouting,
        },
    );
    assert_eq!(
        failure.code,
        "safe_lane_plan_verify_failed_budget_exhausted"
    );
    assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn terminal_verify_failure_uses_session_governor_error_code() {
    let failure = terminal_turn_failure_from_verify_failure(
        "retryable verify failure",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        },
    );
    assert_eq!(
        failure.code,
        "safe_lane_plan_verify_failed_session_governor"
    );
    assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn terminal_plan_failure_uses_session_governor_error_code() {
    let failure = PlanRunFailure::NodeFailed {
        node_id: "tool-1".to_owned(),
        attempts_used: 1,
        last_error_kind: PlanNodeErrorKind::Retryable,
        last_error: "transient".to_owned(),
    };
    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
        source: SafeLaneFailureRouteSource::SessionGovernor,
    };
    assert_eq!(
        route.terminal_plan_failure_code(),
        Some(SafeLaneFailureCode::PlanSessionGovernorNoReplan)
    );
    let result = terminal_turn_result_from_plan_failure_with_route(failure, route);
    let meta = result.failure().expect("failure metadata");
    assert_eq!(meta.code, "safe_lane_plan_session_governor_no_replan");
    assert_eq!(meta.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn decide_safe_lane_verify_failure_action_replans_with_remaining_budget() {
    let decision = decide_safe_lane_verify_failure_action(
        "missing anchors",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        },
    );

    if let SafeLaneRoundDecision::Replan {
        reason,
        next_plan_start_tool_index,
        next_seed_tool_outputs,
    } = decision
    {
        assert_eq!(reason, "verify_failed");
        assert_eq!(next_plan_start_tool_index, 0);
        assert!(next_seed_tool_outputs.is_empty());
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_safe_lane_verify_failure_action_terminalizes_with_governor_code() {
    let decision = decide_safe_lane_verify_failure_action(
        "missing anchors",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        },
    );

    if let SafeLaneRoundDecision::Finalize {
        result: TurnResult::ToolError(failure),
    } = decision
    {
        assert_eq!(
            failure.code,
            "safe_lane_plan_verify_failed_session_governor"
        );
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_safe_lane_plan_failure_action_replans_with_failed_subgraph_cursor() {
    let decision = decide_safe_lane_plan_failure_action(
        PlanRunFailure::NodeFailed {
            node_id: "tool-2".to_owned(),
            attempts_used: 1,
            last_error_kind: PlanNodeErrorKind::Retryable,
            last_error: "transient".to_owned(),
        },
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        },
        1,
        vec!["[ok] {\"path\":\"note.md\"}".to_owned()],
    );

    if let SafeLaneRoundDecision::Replan {
        reason,
        next_plan_start_tool_index,
        next_seed_tool_outputs,
    } = decision
    {
        assert_eq!(
            reason,
            "node_failed node=tool-2 error_kind=Retryable reason=transient"
        );
        assert_eq!(next_plan_start_tool_index, 1);
        assert_eq!(next_seed_tool_outputs.len(), 1);
        assert!(next_seed_tool_outputs[0].contains("note.md"));
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_safe_lane_plan_failure_action_terminalizes_with_backpressure_code() {
    let decision = decide_safe_lane_plan_failure_action(
        PlanRunFailure::NodeFailed {
            node_id: "tool-1".to_owned(),
            attempts_used: 2,
            last_error_kind: PlanNodeErrorKind::Retryable,
            last_error: "transient".to_owned(),
        },
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        },
        0,
        Vec::new(),
    );

    if let SafeLaneRoundDecision::Finalize {
        result: TurnResult::ToolError(failure),
    } = decision
    {
        assert_eq!(failure.code, "safe_lane_plan_backpressure_guard");
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}
