use std::collections::{BTreeMap, BTreeSet};

use loongclaw_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

use crate::KernelContext;

mod claw_import;
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
        "claw_import" | "import_claw" => "claw.import",
        "file_read" => "file.read",
        "file_write" => "file.write",
        "shell_exec" | "shell" => "shell.exec",
        other => other,
    }
}

pub fn is_known_tool_name(raw: &str) -> bool {
    matches!(
        canonical_tool_name(raw),
        "claw.import" | "shell.exec" | "file.read" | "file.write"
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
        "claw.import" => claw_import::execute_claw_import_tool_with_config(request, config),
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
    entries.push(ToolRegistryEntry {
        name: "claw.import",
        description: "Import legacy Claw configs into native LoongClaw settings",
    });
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
    entries.sort_by_key(|entry| entry.name);
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

    tools.push(json!({
        "type": "function",
        "function": {
            "name": "claw_import",
            "description": "Import a legacy Claw workspace or persona into native LoongClaw config with preview or apply mode.",
            "parameters": {
                "type": "object",
                "properties": {
                    "input_path": {
                        "type": "string",
                        "description": "Path to the legacy Claw workspace, config root, or portable import file."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["plan", "apply"],
                        "description": "Use `plan` to preview the nativeized result, or `apply` to write a target config."
                    },
                    "source": {
                        "type": "string",
                        "enum": ["auto", "nanobot", "openclaw", "picoclaw", "zeroclaw", "nanoclaw"],
                        "description": "Optional source hint. Defaults to automatic detection."
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Target config path to write in apply mode."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Overwrite an existing target config when applying. Defaults to false."
                    }
                },
                "required": ["input_path"],
                "additionalProperties": false
            }
        }
    }));

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

    tools.sort_by(|left, right| tool_function_name(left).cmp(tool_function_name(right)));
    tools
}

fn tool_function_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[allow(dead_code)]
fn _shape_examples() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        (
            "claw.import",
            json!({
                "input_path": "/tmp/nanobot-workspace",
                "mode": "plan",
                "source": "auto"
            }),
        ),
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
        assert!(snapshot
            .contains("- claw.import: Import legacy Claw configs into native LoongClaw settings"));
        assert!(snapshot.contains("- file.read: Read file contents"));
        assert!(snapshot.contains("- file.write: Write file contents"));
        assert!(snapshot.contains("- shell.exec: Execute shell commands"));

        // Verify sorted order: claw.import < file.read < file.write < shell.exec
        let lines: Vec<&str> = snapshot.lines().skip(1).collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("- claw.import"));
        assert!(lines[1].starts_with("- file.read"));
        assert!(lines[2].starts_with("- file.write"));
        assert!(lines[3].starts_with("- shell.exec"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn tool_registry_returns_all_known_tools() {
        let entries = tool_registry();
        assert_eq!(entries.len(), 4);
        let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
        assert!(names.contains(&"claw.import"));
        assert!(names.contains(&"shell.exec"));
        assert!(names.contains(&"file.read"));
        assert!(names.contains(&"file.write"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn provider_tool_definitions_are_stable_and_complete() {
        let defs = provider_tool_definitions();
        assert_eq!(defs.len(), 4);

        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            names,
            vec!["claw_import", "file_read", "file_write", "shell_exec"]
        );

        for item in &defs {
            assert_eq!(item["type"], "function");
            assert_eq!(item["function"]["parameters"]["type"], "object");
        }
    }

    #[test]
    fn canonical_tool_name_maps_known_aliases() {
        assert_eq!(canonical_tool_name("claw_import"), "claw.import");
        assert_eq!(canonical_tool_name("file_read"), "file.read");
        assert_eq!(canonical_tool_name("file_write"), "file.write");
        assert_eq!(canonical_tool_name("shell_exec"), "shell.exec");
        assert_eq!(canonical_tool_name("shell"), "shell.exec");
        assert_eq!(canonical_tool_name("file.read"), "file.read");
    }

    #[test]
    fn is_known_tool_name_accepts_canonical_and_alias_forms() {
        assert!(is_known_tool_name("claw.import"));
        assert!(is_known_tool_name("claw_import"));
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

    #[test]
    fn claw_import_plan_mode_returns_nativeized_preview() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-plan");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(root.clone()),
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "plan",
                    "source": "nanobot",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import plan should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool_name"], "claw.import");
        assert_eq!(outcome.payload["mode"], "plan");
        assert_eq!(outcome.payload["source"], "nanobot");
        assert_eq!(
            outcome.payload["config_preview"]["prompt_pack_id"],
            "loongclaw-core-v1"
        );
        assert_eq!(
            outcome.payload["config_preview"]["memory_profile"],
            "profile_plus_window"
        );
        assert!(outcome.payload["config_preview"]["system_prompt_addendum"]
            .as_str()
            .expect("prompt addendum should exist")
            .contains("LoongClaw"));
        assert!(outcome.payload["config_preview"]["profile_note"]
            .as_str()
            .expect("profile note should exist")
            .contains("LoongClaw"));
        assert_eq!(outcome.payload["config_written"], false);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_mode_writes_target_config() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let output_path = root.join("generated").join("loongclaw.toml");
        let config = runtime_config::ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(root.clone()),
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw_import".to_owned(),
                payload: json!({
                    "mode": "apply",
                    "source": "nanobot",
                    "input_path": ".",
                    "output_path": "generated/loongclaw.toml",
                    "force": true
                }),
            },
            &config,
        )
        .expect("claw import apply should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply");
        assert_eq!(outcome.payload["config_written"], true);
        assert_eq!(
            outcome.payload["output_path"]
                .as_str()
                .expect("output path should exist"),
            fs::canonicalize(&output_path)
                .expect("output path should canonicalize")
                .display()
                .to_string()
        );

        let raw = fs::read_to_string(&output_path).expect("output config should exist");
        assert!(raw.contains("prompt_pack_id = \"loongclaw-core-v1\""));
        assert!(raw.contains("profile = \"profile_plus_window\""));
        assert!(raw.contains("LoongClaw"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_discover_mode_returns_detected_sources() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-discover");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(root.clone()),
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "discover",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import discover should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "discover");
        assert_eq!(outcome.payload["sources"][0]["source_id"], "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_plan_many_mode_returns_source_summaries_and_recommendation() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-plan-many");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(root.clone()),
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "plan_many",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import plan_many should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "plan_many");
        assert_eq!(outcome.payload["plans"][0]["source_id"], "openclaw");
        assert_eq!(outcome.payload["recommendation"]["source_id"], "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_merge_profiles_mode_preserves_prompt_owner() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-merge-profiles");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- region: apac\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(root.clone()),
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "merge_profiles",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import merge_profiles should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "merge_profiles");
        assert_eq!(
            outcome.payload["result"]["prompt_owner_source_id"],
            "openclaw"
        );
        assert!(
            outcome.payload["result"]["merged_profile_note"]
                .as_str()
                .expect("merged profile note should be present")
                .contains("region: apac")
        );

        fs::remove_dir_all(&root).ok();
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
