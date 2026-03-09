use std::collections::{BTreeMap, BTreeSet};

use loongclaw_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

use crate::KernelContext;

mod file;
mod kernel_adapter;
pub mod runtime_config;
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
    execute_tool_core_with_config(request, runtime_config::get_tool_runtime_config())
}

pub fn canonical_tool_name(raw: &str) -> &str {
    match raw {
        "file_read" => "file.read",
        "file_write" => "file.write",
        "shell_exec" | "shell" => "shell.exec",
        other => other,
    }
}

pub fn is_known_tool_name(raw: &str) -> bool {
    matches!(
        canonical_tool_name(raw),
        "shell.exec" | "file.read" | "file.write"
    )
}

pub fn execute_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };
    match canonical_name {
        "shell.exec" => shell::execute_shell_tool_with_config(request, config),
        "file.read" => file::execute_file_read_tool_with_config(request, config),
        "file.write" => file::execute_file_write_tool_with_config(request, config),
        _ => Err(format!(
            "tool_not_found: unknown tool `{}`",
            request.tool_name
        )),
    }
}

/// Tool registry entry for capability snapshot disclosure.
#[derive(Debug, Clone)]
pub struct ToolRegistryEntry {
    pub name: &'static str,
    pub description: &'static str,
}

/// Returns a sorted list of all registered tools, gated by feature flags.
pub fn tool_registry() -> Vec<ToolRegistryEntry> {
    let mut entries = Vec::new();
    #[cfg(feature = "tool-file")]
    {
        entries.push(ToolRegistryEntry {
            name: "file.read",
            description: "Read file contents",
        });
        entries.push(ToolRegistryEntry {
            name: "file.write",
            description: "Write file contents",
        });
    }
    #[cfg(feature = "tool-shell")]
    {
        entries.push(ToolRegistryEntry {
            name: "shell.exec",
            description: "Execute shell commands",
        });
    }
    entries
}

/// Produce a deterministic text block listing available tools,
/// suitable for appending to the system prompt.
pub fn capability_snapshot() -> String {
    let entries = tool_registry();
    let mut lines = vec!["[available_tools]".to_owned()];
    for entry in &entries {
        lines.push(format!("- {}: {}", entry.name, entry.description));
    }
    lines.join("\n")
}

/// Provider request tool schema for function-calling capable models.
///
/// The output shape matches OpenAI-compatible `tools=[{type:function,...}]`.
/// Order is deterministic for stable prompting/tests.
pub fn provider_tool_definitions() -> Vec<Value> {
    let mut tools = Vec::new();

    #[cfg(feature = "tool-file")]
    {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "file_read",
                "description": "Read file contents",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to read (absolute or relative to configured file root)."
                        },
                        "max_bytes": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 8_388_608,
                            "description": "Optional read limit in bytes. Defaults to 1048576."
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }
            }
        }));
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "file_write",
                "description": "Write file contents",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to write (absolute or relative to configured file root)."
                        },
                        "content": {
                            "type": "string",
                            "description": "File content to write."
                        },
                        "create_dirs": {
                            "type": "boolean",
                            "description": "Create parent directories when missing. Defaults to true."
                        }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }
            }
        }));
    }

    #[cfg(feature = "tool-shell")]
    {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "shell_exec",
                "description": "Execute shell commands",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Executable command name. Must be allowlisted."
                        },
                        "args": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Optional command arguments."
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Optional working directory."
                        }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }
            }
        }));
    }

    tools
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
    fn capability_snapshot_is_deterministic() {
        let snapshot = capability_snapshot();
        assert!(snapshot.starts_with("[available_tools]"));

        // Verify determinism: two calls produce identical output.
        let snapshot2 = capability_snapshot();
        assert_eq!(snapshot, snapshot2);
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn capability_snapshot_lists_all_tools_when_all_features_enabled() {
        let snapshot = capability_snapshot();
        assert!(snapshot.contains("- file.read: Read file contents"));
        assert!(snapshot.contains("- file.write: Write file contents"));
        assert!(snapshot.contains("- shell.exec: Execute shell commands"));

        // Verify sorted order: file.read < file.write < shell.exec
        let lines: Vec<&str> = snapshot.lines().skip(1).collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("- file.read"));
        assert!(lines[1].starts_with("- file.write"));
        assert!(lines[2].starts_with("- shell.exec"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn tool_registry_returns_all_known_tools() {
        let entries = tool_registry();
        assert_eq!(entries.len(), 3);
        let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
        assert!(names.contains(&"shell.exec"));
        assert!(names.contains(&"file.read"));
        assert!(names.contains(&"file.write"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn provider_tool_definitions_are_stable_and_complete() {
        let defs = provider_tool_definitions();
        assert_eq!(defs.len(), 3);

        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(names, vec!["file_read", "file_write", "shell_exec"]);

        for item in &defs {
            assert_eq!(item["type"], "function");
            assert_eq!(item["function"]["parameters"]["type"], "object");
        }
    }

    #[test]
    fn canonical_tool_name_maps_known_aliases() {
        assert_eq!(canonical_tool_name("file_read"), "file.read");
        assert_eq!(canonical_tool_name("file_write"), "file.write");
        assert_eq!(canonical_tool_name("shell_exec"), "shell.exec");
        assert_eq!(canonical_tool_name("shell"), "shell.exec");
        assert_eq!(canonical_tool_name("file.read"), "file.read");
    }

    #[test]
    fn is_known_tool_name_accepts_canonical_and_alias_forms() {
        assert!(is_known_tool_name("file.read"));
        assert!(is_known_tool_name("file_read"));
        assert!(is_known_tool_name("file.write"));
        assert!(is_known_tool_name("file_write"));
        assert!(is_known_tool_name("shell.exec"));
        assert!(is_known_tool_name("shell_exec"));
        assert!(is_known_tool_name("shell"));
        assert!(!is_known_tool_name("nonexistent.tool"));
    }

    #[test]
    fn unknown_tool_returns_hard_error_code() {
        let err = execute_tool_core(ToolCoreRequest {
            tool_name: "unknown".to_owned(),
            payload: json!({"hello":"world"}),
        })
        .expect_err("unknown tool should return an error");
        assert!(
            err.contains("tool_not_found"),
            "error should contain tool_not_found, got: {err}"
        );
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
        kernel.register_core_tool_adapter(MvpToolAdapter::new());
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let caps = BTreeSet::from([Capability::InvokeTool]);
        // Use an unknown tool name — it should propagate as an error through the adapter
        let request = ToolCoreRequest {
            tool_name: "noop".to_owned(),
            payload: json!({"key": "value"}),
        };
        let err = kernel
            .execute_tool_core("test-pack", &token, &caps, None, request)
            .await
            .expect_err("unknown tool via MvpToolAdapter should fail");
        assert!(
            format!("{err}").contains("tool_not_found"),
            "error should contain tool_not_found, got: {err}"
        );
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
