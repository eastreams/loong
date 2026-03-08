use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
use loongclaw_kernel::{
    CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, MemoryCoreOutcome,
    MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
};
use serde_json::{json, Value};

use super::super::config::{
    CliChannelConfig, FeishuChannelConfig, LoongClawConfig, MemoryConfig, ProviderConfig,
    TelegramChannelConfig, ToolConfig,
};
use super::persistence::format_provider_error_reply;
use super::runtime::DefaultConversationRuntime;
use super::*;
use crate::CliResult;
use crate::KernelContext;

struct FakeRuntime {
    seed_messages: Vec<Value>,
    completion: Result<String, String>,
    persisted: Mutex<Vec<(String, String, String)>>,
    requested_messages: Mutex<Vec<Value>>,
}

impl FakeRuntime {
    fn new(seed_messages: Vec<Value>, completion: Result<String, String>) -> Self {
        Self {
            seed_messages,
            completion,
            persisted: Mutex::new(Vec::new()),
            requested_messages: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ConversationRuntime for FakeRuntime {
    fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        Ok(self.seed_messages.clone())
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<String> {
        *self.requested_messages.lock().expect("request lock") = messages.to_vec();
        self.completion.clone().map_err(|error| error.to_owned())
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        self.persisted.lock().expect("persist lock").push((
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
        ));
        Ok(())
    }
}

fn test_config() -> LoongClawConfig {
    LoongClawConfig {
        provider: ProviderConfig::default(),
        cli: CliChannelConfig::default(),
        telegram: TelegramChannelConfig::default(),
        feishu: FeishuChannelConfig::default(),
        tools: ToolConfig::default(),
        memory: MemoryConfig::default(),
    }
}

#[tokio::test]
async fn handle_turn_with_runtime_success_persists_user_and_assistant_turns() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let orchestrator = ConversationOrchestrator::new();
    let reply = orchestrator
        .handle_turn_with_runtime(
            &test_config(),
            "session-1",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");

    let requested = runtime.requested_messages.lock().expect("requested lock");
    assert_eq!(requested.len(), 2);
    assert_eq!(requested[1]["role"], "user");
    assert_eq!(requested[1]["content"], "hello");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(
        persisted[0],
        (
            "session-1".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        persisted[1],
        (
            "session-1".to_owned(),
            "assistant".to_owned(),
            "assistant-reply".to_owned(),
        )
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_propagates_error_without_persisting() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let orchestrator = ConversationOrchestrator::new();
    let error = orchestrator
        .handle_turn_with_runtime(
            &test_config(),
            "session-2",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect_err("propagate mode should return error");

    assert!(error.contains("timeout"));
    assert!(runtime.persisted.lock().expect("persisted lock").is_empty());
}

#[tokio::test]
async fn handle_turn_with_runtime_inline_mode_returns_synthetic_reply_and_persists() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let orchestrator = ConversationOrchestrator::new();
    let output = orchestrator
        .handle_turn_with_runtime(
            &test_config(),
            "session-3",
            "hello",
            ProviderErrorMode::InlineMessage,
            &runtime,
            None,
        )
        .await
        .expect("inline mode should return synthetic reply");

    assert_eq!(output, "[provider_error] timeout");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(
        persisted[0],
        (
            "session-3".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        persisted[1],
        (
            "session-3".to_owned(),
            "assistant".to_owned(),
            "[provider_error] timeout".to_owned(),
        )
    );
}

#[test]
fn format_provider_error_reply_is_stable() {
    let output = format_provider_error_reply("timeout");
    assert_eq!(output, "[provider_error] timeout");
}

// --- Kernel-routed memory tests ---

fn build_kernel_context(
    audit: Arc<InMemoryAuditSink>,
) -> (KernelContext, Arc<Mutex<Vec<MemoryCoreRequest>>>) {
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::MemoryWrite, Capability::MemoryRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");

    let invocations = Arc::new(Mutex::new(Vec::new()));
    let adapter = SharedTestMemoryAdapter {
        invocations: invocations.clone(),
    };
    kernel.register_core_memory_adapter(adapter);
    kernel
        .set_default_core_memory_adapter("test-memory-shared")
        .expect("set default memory adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    (ctx, invocations)
}

struct SharedTestMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
}

#[async_trait]
impl CoreMemoryAdapter for SharedTestMemoryAdapter {
    fn name(&self) -> &str {
        "test-memory-shared"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request);
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({}),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persist_turn_routes_through_kernel_when_context_provided() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context(audit.clone());

    let runtime = DefaultConversationRuntime;
    runtime
        .persist_turn("session-k1", "user", "kernel-hello", Some(&ctx))
        .await
        .expect("persist via kernel");

    // Verify the memory adapter received the request.
    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, "append_turn");
    assert_eq!(captured[0].payload["session_id"], "session-k1");
    assert_eq!(captured[0].payload["role"], "user");
    assert_eq!(captured[0].payload["content"], "kernel-hello");

    // Verify audit events contain a memory plane invocation.
    let events = audit.snapshot();
    let has_memory_plane = events.iter().any(|event| {
        matches!(
            &event.kind,
            loongclaw_kernel::AuditEventKind::PlaneInvoked {
                plane: loongclaw_contracts::ExecutionPlane::Memory,
                ..
            }
        )
    });
    assert!(
        has_memory_plane,
        "audit should contain memory plane invocation"
    );
}
