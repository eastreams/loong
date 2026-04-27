use super::*;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

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

#[test]
fn explicit_skill_activation_parser_extracts_skill_and_request() {
    assert_eq!(
        parse_explicit_skill_activation_input("  $Demo.Skill summarize release notes"),
        Some(ExplicitSkillActivationInput {
            skill_id: "demo-skill".to_owned(),
            followup_request: "summarize release notes".to_owned(),
        })
    );
}

#[test]
fn explicit_skill_activation_parser_generates_followup_when_request_missing() {
    let parsed =
        parse_explicit_skill_activation_input("$release-guard").expect("explicit activation");
    assert_eq!(parsed.skill_id, "release-guard");
    assert!(
        parsed
            .followup_request
            .contains("Confirm activation briefly and ask what to do next."),
        "missing-request activation should synthesize a followup prompt: {parsed:?}"
    );
}

#[test]
fn explicit_skill_activation_parser_ignores_non_prefix_mentions() {
    assert_eq!(
        parse_explicit_skill_activation_input("please use $release-guard"),
        None
    );
    assert_eq!(
        parse_explicit_skill_activation_input("$release-guard, summarize"),
        None
    );
}

#[cfg(feature = "memory-sqlite")]
#[derive(Default)]
pub(super) struct ApprovalControlRuntime {
    pub(super) bootstrap_calls: StdMutex<usize>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for ApprovalControlRuntime {
    async fn build_messages(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok("approval handled".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
pub(super) struct CoreReplayRuntime;

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for CoreReplayRuntime {
    fn session_context(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        Err("session_context should not be called for core approval replay".to_owned())
    }

    async fn build_messages(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Err("build_messages should not run during core approval replay".to_owned())
    }

    async fn request_completion(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Err("request_completion should not run during core approval replay".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
pub(super) fn sqlite_memory_config(label: &str) -> SessionStoreConfig {
    let path = unique_sqlite_path(label);
    let _ = std::fs::remove_file(&path);
    let mut config = LoongConfig::default();
    config.memory.sqlite_path = path.display().to_string();
    store::session_store_config_from_memory_config(&config.memory)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn seed_pending_approval_request(
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
pub(super) fn unique_workspace_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loong-turn-coordinator-workspace-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[cfg(feature = "memory-sqlite")]
#[derive(Default)]
pub(super) struct RecordingCompactRuntime {
    pub(super) compact_calls: StdMutex<usize>,
}

#[derive(Default)]
pub(super) struct ExplicitSkillActivationRuntime {
    pub(super) completion_messages: StdMutex<Vec<Value>>,
    pub(super) persisted_turns: StdMutex<Vec<(String, String)>>,
    pub(super) bootstrap_calls: StdMutex<usize>,
}

#[async_trait]
impl ConversationRuntime for ExplicitSkillActivationRuntime {
    async fn build_messages(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "explicit skill activation test"
        })])
    }

    async fn request_completion(
        &self,
        _config: &LoongConfig,
        messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let mut stored = self
            .completion_messages
            .lock()
            .expect("completion messages lock");
        *stored = messages.to_vec();
        Ok("explicit activation handled".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn should not run for explicit skill activation control turns")
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn_streaming should not run for explicit skill activation control turns")
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        role: &str,
        content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        let mut stored = self.persisted_turns.lock().expect("persisted turns lock");
        stored.push((role.to_owned(), content.to_owned()));
        Ok(())
    }

    async fn bootstrap(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<crate::conversation::context_engine::ContextEngineBootstrapResult> {
        let mut calls = self.bootstrap_calls.lock().expect("bootstrap lock");
        *calls += 1;
        Ok(Default::default())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_explicit_skill_activation_prefix_injects_skill_context() {
    let workspace_root =
        crate::test_support::unique_temp_dir("turn-coordinator-explicit-skill-activation");
    std::fs::create_dir_all(workspace_root.join(".loong/skills/demo-skill"))
        .expect("create skill root");
    std::fs::write(
            workspace_root.join(".loong/skills/demo-skill/SKILL.md"),
            "---\nname: demo-skill\ndescription: Summarize notes with release discipline.\n---\n\n# Demo Skill\n\nFollow the managed skill instruction before answering.\n",
        )
        .expect("write skill");

    let runtime = ExplicitSkillActivationRuntime::default();
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = LoongConfig::default();
    config.external_skills.enabled = true;
    config.tools.file_root = Some(workspace_root.display().to_string());

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-explicit-skill-activation",
            "$demo-skill summarize the changelog",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("explicit activation turn should succeed");

    assert_eq!(reply, "explicit activation handled");

    let messages = runtime
        .completion_messages
        .lock()
        .expect("completion messages lock")
        .clone();
    let injected_skill = messages
        .iter()
        .find(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("External skill `demo-skill`"))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("skill system message should exist: {messages:?}"));
    assert!(
        injected_skill.contains("External skill `demo-skill`"),
        "explicit activation should inject skill context: {injected_skill}"
    );
    assert!(
        injected_skill.contains("Follow the managed skill instruction before answering."),
        "skill system message should include loaded instructions: {injected_skill}"
    );
    let followup_prompt = messages
        .iter()
        .find(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("Original request:"))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("followup prompt should exist: {messages:?}"));
    assert!(
        followup_prompt.contains("Original request:\nsummarize the changelog"),
        "explicit activation should strip the $skill prefix from the forwarded request: {followup_prompt}"
    );
    assert!(
        !followup_prompt.contains("$demo-skill"),
        "followup prompt should not leak the explicit activation token: {followup_prompt}"
    );

    let persisted_turns = runtime
        .persisted_turns
        .lock()
        .expect("persisted turns lock")
        .clone();
    assert!(
        persisted_turns
            .iter()
            .any(|(role, content)| role == "user" && content == "summarize the changelog"),
        "persisted turns should store the forwarded request without the activation token: {persisted_turns:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for RecordingCompactRuntime {
    async fn build_messages(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }

    async fn request_completion(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok(String::new())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
pub(super) struct CompactSessionBuildMessagesRuntime {
    pub(super) session_tool_view: crate::tools::ToolView,
    pub(super) build_messages_calls: StdMutex<Vec<(bool, crate::tools::ToolView)>>,
    pub(super) fail_after_first_readback: bool,
}

#[cfg(feature = "memory-sqlite")]
impl CompactSessionBuildMessagesRuntime {
    pub(super) fn new(
        session_tool_view: crate::tools::ToolView,
        fail_after_first_readback: bool,
    ) -> Self {
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
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok(String::new())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
        _config: &LoongConfig,
        _session_id: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct ObserverStreamingRuntime {
    pub(super) streaming_calls: StdMutex<usize>,
}

#[async_trait]
impl ConversationRuntime for ObserverStreamingRuntime {
    async fn build_messages(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok("completion".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
pub(super) struct ObserverFallbackRuntime {
    pub(super) request_turn_calls: StdMutex<usize>,
    pub(super) request_turn_streaming_calls: StdMutex<usize>,
}

#[async_trait]
impl ConversationRuntime for ObserverFallbackRuntime {
    async fn build_messages(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok("completion".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
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
        _config: &LoongConfig,
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
pub(super) struct RecordingTurnObserver {
    pub(super) phase_events: StdMutex<Vec<ConversationTurnPhaseEvent>>,
    pub(super) tool_events: StdMutex<Vec<ConversationTurnToolEvent>>,
    pub(super) token_events: StdMutex<Vec<crate::acp::StreamingTokenEvent>>,
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
