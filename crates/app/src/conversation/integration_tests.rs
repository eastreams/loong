use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind};
use loongclaw_kernel::{
    FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine, VerticalPackManifest,
};

use super::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
use crate::context::KernelContext;
use crate::tools::MvpToolAdapter;
use crate::tools::runtime_config::ToolRuntimeConfig;

/// Monotonic counter for unique harness IDs (avoids temp dir collisions).
static HARNESS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ergonomic builder for constructing fake `ProviderTurn` responses in tests.
pub(crate) struct FakeProviderBuilder {
    text: String,
    tool_calls: Vec<(String, serde_json::Value)>,
}

impl FakeProviderBuilder {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            tool_calls: Vec::new(),
        }
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.text = text.to_owned();
        self
    }

    pub fn with_tool_call(mut self, tool_name: &str, args: serde_json::Value) -> Self {
        self.tool_calls.push((tool_name.to_owned(), args));
        self
    }

    pub fn build(self) -> ProviderTurn {
        let tool_intents = self
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(i, (name, args))| ToolIntent {
                tool_name: name,
                args_json: args,
                source: "fake_provider".to_owned(),
                session_id: "test-session".to_owned(),
                turn_id: "test-turn".to_owned(),
                tool_call_id: format!("call-{i}"),
            })
            .collect();

        ProviderTurn {
            assistant_text: self.text,
            tool_intents,
            raw_meta: serde_json::Value::Null,
        }
    }
}

/// Integration test harness composing real kernel + real tools + fake provider.
///
/// Each harness gets:
/// - A unique temp dir (no collision between parallel tests)
/// - An `MvpToolAdapter` with injected `ToolRuntimeConfig` (no OnceLock race)
/// - A real `InMemoryAuditSink` for audit assertions
/// - `max_tool_steps = 1`
#[allow(dead_code)]
pub(crate) struct TurnTestHarness {
    pub engine: TurnEngine,
    pub kernel_ctx: KernelContext,
    pub audit: Arc<InMemoryAuditSink>,
    pub temp_dir: PathBuf,
}

impl TurnTestHarness {
    pub fn new() -> Self {
        Self::with_capabilities(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]))
    }

    pub fn with_capabilities(capabilities: BTreeSet<Capability>) -> Self {
        Self::with_tool_config(capabilities, ToolRuntimeConfig::default())
    }

    /// Construct a harness with a caller-supplied `ToolRuntimeConfig`.
    /// Use this when a test needs specific allow/deny/approval lists rather
    /// than the generic defaults.
    pub fn with_tool_config(
        capabilities: BTreeSet<Capability>,
        tool_config_override: ToolRuntimeConfig,
    ) -> Self {
        let id = HARNESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-integ-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        // Merge the caller's overrides with the unique temp dir as file_root.
        let tool_config = ToolRuntimeConfig {
            file_root: Some(temp_dir.clone()),
            config_path: Some(temp_dir.join("loongclaw.toml")),
            ..tool_config_override
        };

        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel =
            LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: capabilities,
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");
        kernel.register_core_tool_adapter(MvpToolAdapter::with_config(tool_config.clone()));
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default adapter");

        // Register policy extensions for unified security enforcement.
        // Policy rules come exclusively from the runtime config; no hardcoded
        // lists are injected here.
        kernel.register_policy_extension(
            crate::tools::shell_policy_ext::ToolPolicyExtension::from_config(&tool_config),
        );
        kernel.register_policy_extension(crate::tools::file_policy_ext::FilePolicyExtension::new(
            tool_config.file_root,
        ));

        #[cfg(feature = "memory-sqlite")]
        {
            use crate::memory::runtime_config::MemoryRuntimeConfig;
            let memory_config = MemoryRuntimeConfig {
                sqlite_path: Some(temp_dir.join("memory.sqlite3")),
                ..MemoryRuntimeConfig::default()
            };
            kernel.register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(
                memory_config,
            ));
            kernel
                .set_default_core_memory_adapter("mvp-memory")
                .expect("set default memory adapter");
        }

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let ctx = KernelContext {
            kernel: Arc::new(kernel),
            token,
        };

        Self {
            engine: TurnEngine::new(1),
            kernel_ctx: ctx,
            audit,
            temp_dir,
        }
    }

    /// Execute a provider turn through the full TurnEngine path.
    #[allow(dead_code)]
    pub async fn execute(&self, turn: &ProviderTurn) -> TurnResult {
        self.engine.execute_turn(turn, Some(&self.kernel_ctx)).await
    }
}

impl Drop for TurnTestHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fake_provider_builder_text_only() {
        let turn = FakeProviderBuilder::new().with_text("hello world").build();
        assert_eq!(turn.assistant_text, "hello world");
        assert!(turn.tool_intents.is_empty());
    }

    #[test]
    fn fake_provider_builder_with_tool_call() {
        let turn = FakeProviderBuilder::new()
            .with_text("checking file")
            .with_tool_call("file.read", json!({"path": "test.txt"}))
            .build();
        assert_eq!(turn.assistant_text, "checking file");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "file.read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "test.txt"}));
        assert!(!turn.tool_intents[0].tool_call_id.is_empty());
    }

    #[test]
    fn fake_provider_builder_unique_tool_call_ids() {
        let turn = FakeProviderBuilder::new()
            .with_tool_call("file.read", json!({"path": "a.txt"}))
            .with_tool_call("file.read", json!({"path": "b.txt"}))
            .build();
        assert_eq!(turn.tool_intents.len(), 2);
        assert_ne!(
            turn.tool_intents[0].tool_call_id,
            turn.tool_intents[1].tool_call_id
        );
    }

    #[test]
    fn harness_builds_with_invoke_tool_capability() {
        let harness = TurnTestHarness::new();
        assert!(
            harness
                .kernel_ctx
                .token
                .allowed_capabilities
                .contains(&Capability::InvokeTool)
        );
        assert!(harness.temp_dir.exists());
    }

    #[test]
    fn harness_temp_dirs_are_unique() {
        let h1 = TurnTestHarness::new();
        let h2 = TurnTestHarness::new();
        assert_ne!(h1.temp_dir, h2.temp_dir);
    }

    // ── Real-execution integration tests ──────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_file_read_returns_real_content() {
        let harness = TurnTestHarness::new();
        std::fs::write(
            harness.temp_dir.join("greeting.txt"),
            "hello from integration test",
        )
        .expect("seed file");

        let turn = FakeProviderBuilder::new()
            .with_tool_call("file.read", json!({"path": "greeting.txt"}))
            .build();
        let result = harness.execute(&turn).await;

        #[allow(clippy::wildcard_enum_match_arm)]
        match result {
            TurnResult::FinalText(text) => {
                assert!(
                    text.contains("hello from integration test"),
                    "expected file content in output, got: {text}"
                );
            }
            other => panic!("expected FinalText, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_file_write_then_read_round_trip() {
        let harness = TurnTestHarness::new();

        // Write
        let write_turn = FakeProviderBuilder::new()
            .with_tool_call(
                "file.write",
                json!({"path": "round-trip.txt", "content": "written by tool"}),
            )
            .build();
        let write_result = harness.execute(&write_turn).await;
        #[allow(clippy::wildcard_enum_match_arm)]
        match &write_result {
            TurnResult::FinalText(text) => {
                assert!(
                    text.contains("bytes_written") || text.contains("round-trip.txt"),
                    "expected write metadata in write result, got: {text}"
                );
            }
            other => panic!("expected FinalText for write, got: {other:?}"),
        }

        // Read back
        let read_turn = FakeProviderBuilder::new()
            .with_tool_call("file.read", json!({"path": "round-trip.txt"}))
            .build();
        let read_result = harness.execute(&read_turn).await;
        #[allow(clippy::wildcard_enum_match_arm)]
        match read_result {
            TurnResult::FinalText(text) => {
                assert!(
                    text.contains("written by tool"),
                    "expected written content in read result, got: {text}"
                );
            }
            other => panic!("expected FinalText for read, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_shell_exec_echo() {
        let harness = TurnTestHarness::with_tool_config(
            BTreeSet::from([
                Capability::InvokeTool,
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
            ToolRuntimeConfig {
                shell_allow: BTreeSet::from(["echo".to_owned()]),
                ..ToolRuntimeConfig::default()
            },
        );

        let turn = FakeProviderBuilder::new()
            .with_tool_call("shell.exec", json!({"command": "echo", "args": ["hello"]}))
            .build();
        let result = harness.execute(&turn).await;

        #[allow(clippy::wildcard_enum_match_arm)]
        match result {
            TurnResult::FinalText(text) => {
                assert!(
                    text.contains("hello"),
                    "expected 'hello' in output, got: {text}"
                );
            }
            other => panic!("expected FinalText, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_shell_exec_blocked_command() {
        let harness = TurnTestHarness::with_tool_config(
            BTreeSet::from([Capability::InvokeTool]),
            ToolRuntimeConfig {
                shell_deny: BTreeSet::from(["rm".to_owned()]),
                ..ToolRuntimeConfig::default()
            },
        );

        let turn = FakeProviderBuilder::new()
            .with_tool_call("shell.exec", json!({"command": "rm", "args": ["-rf", "/"]}))
            .build();
        let result = harness.execute(&turn).await;

        #[allow(clippy::wildcard_enum_match_arm)]
        match result {
            TurnResult::ToolDenied(err) => {
                assert!(
                    err.contains("blocked by shell policy"),
                    "expected policy-block reason, got: {err}"
                );
            }
            other => panic!("expected ToolDenied with policy reason, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_file_read_sandbox_rejects_path_escape() {
        let harness = TurnTestHarness::new();

        let turn = FakeProviderBuilder::new()
            .with_tool_call("file.read", json!({"path": "../../../etc/passwd"}))
            .build();
        let result = harness.execute(&turn).await;

        #[allow(clippy::wildcard_enum_match_arm)]
        match result {
            TurnResult::ToolDenied(err) => {
                assert!(
                    err.contains("escapes"),
                    "expected 'escapes' in error, got: {err}"
                );
            }
            other => panic!("expected ToolDenied with 'escapes', got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_missing_capability_denies_tool() {
        let harness = TurnTestHarness::with_capabilities(BTreeSet::from([Capability::MemoryRead]));

        let turn = FakeProviderBuilder::new()
            .with_tool_call("file.read", json!({"path": "anything.txt"}))
            .build();
        let result = harness.execute(&turn).await;

        #[allow(clippy::wildcard_enum_match_arm)]
        match result {
            TurnResult::ToolDenied(reason) => {
                let lower = reason.to_lowercase();
                assert!(
                    lower.contains("capability") || lower.contains("denied"),
                    "expected capability/denied in reason, got: {reason}"
                );
            }
            other => panic!("expected ToolDenied, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_audit_captures_tool_plane_invocation() {
        let harness = TurnTestHarness::with_tool_config(
            BTreeSet::from([
                Capability::InvokeTool,
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
            ToolRuntimeConfig {
                shell_allow: BTreeSet::from(["echo".to_owned()]),
                ..ToolRuntimeConfig::default()
            },
        );

        let turn = FakeProviderBuilder::new()
            .with_tool_call("shell.exec", json!({"command": "echo", "args": ["audit"]}))
            .build();
        let result = harness.execute(&turn).await;
        assert!(
            matches!(result, TurnResult::FinalText(_)),
            "expected FinalText, got: {result:?}"
        );

        let events = harness.audit.snapshot();
        let has_tool_plane = events.iter().any(|event| {
            matches!(
                &event.kind,
                loongclaw_kernel::AuditEventKind::PlaneInvoked {
                    plane: loongclaw_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });
        assert!(
            has_tool_plane,
            "audit should contain PlaneInvoked{{Tool}} event, got: {:?}",
            events
                .iter()
                .map(|e| format!("{:?}", e.kind))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_malformed_tool_args_returns_error() {
        let harness = TurnTestHarness::new();

        let turn = FakeProviderBuilder::new()
            .with_tool_call("file.read", json!("not an object"))
            .build();
        let result = harness.execute(&turn).await;

        #[allow(clippy::wildcard_enum_match_arm)]
        match result {
            TurnResult::ToolError(err) => {
                assert!(
                    err.contains("payload must be an object"),
                    "expected 'payload must be an object' in error, got: {err}"
                );
            }
            other => {
                panic!("expected ToolError with 'payload must be an object', got: {other:?}");
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integ_file_write_denied_without_capability() {
        let harness = TurnTestHarness::with_capabilities(BTreeSet::from([Capability::InvokeTool]));

        let turn = FakeProviderBuilder::new()
            .with_tool_call(
                "file.write",
                json!({"path": "test.txt", "content": "hello"}),
            )
            .build();
        let result = harness.execute(&turn).await;

        match result {
            TurnResult::ToolDenied(err) => {
                assert!(
                    err.contains("FilesystemWrite"),
                    "expected 'FilesystemWrite' in reason, got: {err}"
                );
            }
            other @ (TurnResult::FinalText(_)
            | TurnResult::NeedsApproval(_)
            | TurnResult::ToolError(_)
            | TurnResult::ProviderError(_)) => {
                panic!("expected ToolDenied with FilesystemWrite, got: {other:?}")
            }
        }
    }
}
