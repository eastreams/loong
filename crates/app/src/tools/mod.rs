use std::collections::{BTreeMap, BTreeSet};

use loongclaw_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

use crate::KernelContext;

mod file;
mod kernel_adapter;
mod shell;

pub use kernel_adapter::MvpToolAdapter;

/// Execute a tool request, optionally routing through the kernel for
/// policy enforcement and audit recording.
///
/// When `kernel_ctx` is `Some`, the request is dispatched via
/// `kernel.execute_tool_core` which enforces `InvokeTool` capability
/// and records audit events.  When `None`, the request falls through
/// to the direct `execute_tool_core` path.
pub async fn execute_tool(
    request: ToolCoreRequest,
    kernel_ctx: Option<&KernelContext>,
) -> Result<ToolCoreOutcome, String> {
    match kernel_ctx {
        Some(ctx) => {
            let caps = BTreeSet::from([Capability::InvokeTool]);
            ctx.kernel
                .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|e| format!("{e}"))
        }
        None => execute_tool_core(request),
    }
}

pub fn execute_tool_core(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    match request.tool_name.as_str() {
        "shell.exec" | "shell_exec" | "shell" => shell::execute_shell_tool(request),
        "file.read" | "file_read" => file::execute_file_read_tool(request),
        "file.write" | "file_write" => file::execute_file_write_tool(request),
        _ => Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "payload": request.payload,
            }),
        }),
    }
}

#[allow(dead_code)]
fn _shape_examples() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        (
            "shell.exec",
            json!({
                "command": "echo",
                "args": ["hello"]
            }),
        ),
        (
            "file.read",
            json!({
                "path": "README.md",
                "max_bytes": 4096
            }),
        ),
        (
            "file.write",
            json!({
                "path": "notes.txt",
                "content": "hello",
                "create_dirs": true
            }),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tool_keeps_backward_compatible_payload_shape() {
        let outcome = execute_tool_core(ToolCoreRequest {
            tool_name: "unknown".to_owned(),
            payload: json!({"hello":"world"}),
        })
        .expect("unknown tool should fallback to echo behavior");
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["adapter"], "core-tools");
    }

    // --- Kernel-routed tool tests ---

    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use loongclaw_contracts::{ExecutionRoute, HarnessKind, ToolPlaneError};
    use loongclaw_kernel::{
        CoreToolAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine,
        VerticalPackManifest,
    };

    struct SharedTestToolAdapter {
        invocations: Arc<Mutex<Vec<ToolCoreRequest>>>,
    }

    #[async_trait]
    impl CoreToolAdapter for SharedTestToolAdapter {
        fn name(&self) -> &str {
            "test-tool-shared"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    fn build_tool_kernel_context(
        audit: Arc<InMemoryAuditSink>,
        capabilities: BTreeSet<Capability>,
    ) -> (KernelContext, Arc<Mutex<Vec<ToolCoreRequest>>>) {
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
            granted_capabilities: capabilities,
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");

        let invocations = Arc::new(Mutex::new(Vec::new()));
        let adapter = SharedTestToolAdapter {
            invocations: invocations.clone(),
        };
        kernel.register_core_tool_adapter(adapter);
        kernel
            .set_default_core_tool_adapter("test-tool-shared")
            .expect("set default tool adapter");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let ctx = KernelContext {
            kernel: Arc::new(kernel),
            token,
        };

        (ctx, invocations)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_call_through_kernel_records_audit() {
        let audit = Arc::new(InMemoryAuditSink::default());
        let (ctx, invocations) =
            build_tool_kernel_context(audit.clone(), BTreeSet::from([Capability::InvokeTool]));

        let request = ToolCoreRequest {
            tool_name: "echo".to_owned(),
            payload: json!({"msg": "hello"}),
        };
        let outcome = execute_tool(request, Some(&ctx))
            .await
            .expect("tool call via kernel should succeed");
        assert_eq!(outcome.status, "ok");

        // Verify the tool adapter received the request.
        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].tool_name, "echo");

        // Verify audit events contain a tool plane invocation.
        let events = audit.snapshot();
        let has_tool_plane = events.iter().any(|event| {
            matches!(
                &event.kind,
                loongclaw_kernel::AuditEventKind::PlaneInvoked {
                    plane: loongclaw_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });
        assert!(has_tool_plane, "audit should contain tool plane invocation");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mvp_tool_adapter_routes_through_kernel() {
        use kernel_adapter::MvpToolAdapter;

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
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");
        kernel.register_core_tool_adapter(MvpToolAdapter);
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let caps = BTreeSet::from([Capability::InvokeTool]);
        // Use an unknown tool name so it hits the fallback path without side effects
        let request = ToolCoreRequest {
            tool_name: "noop".to_owned(),
            payload: json!({"key": "value"}),
        };
        let outcome = kernel
            .execute_tool_core("test-pack", &token, &caps, None, request)
            .await
            .expect("tool call via MvpToolAdapter should succeed");
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["adapter"], "core-tools");

        let events = audit.snapshot();
        let has_tool_plane = events.iter().any(|event| {
            matches!(
                &event.kind,
                loongclaw_kernel::AuditEventKind::PlaneInvoked {
                    plane: loongclaw_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });
        assert!(has_tool_plane, "audit should contain tool plane invocation");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_call_through_kernel_denied_without_capability() {
        let audit = Arc::new(InMemoryAuditSink::default());
        // Grant MemoryRead only — InvokeTool is missing.
        let (ctx, _invocations) =
            build_tool_kernel_context(audit, BTreeSet::from([Capability::MemoryRead]));

        let request = ToolCoreRequest {
            tool_name: "echo".to_owned(),
            payload: json!({"msg": "hello"}),
        };
        let err = execute_tool(request, Some(&ctx))
            .await
            .expect_err("should be denied without InvokeTool capability");

        // The error message should indicate a policy/capability denial.
        assert!(
            err.contains("denied") || err.contains("capability") || err.contains("Capability"),
            "error should mention denial or capability, got: {err}"
        );
    }
}
