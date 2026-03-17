# Tool Discovery Architecture Implementation Plan

**Goal:** Replace the current static provider-wide tool exposure with a discovery-first runtime where only `tool_search` and `tool_invoke` are provider-callable, while non-core tools remain executable behind a lease-validated dispatcher.

**Architecture:** Introduce an app-native tool catalog that separates provider-core tools from discoverable tools. Generate the provider schema and capability snapshot from that catalog, add first-class `tool_search` and `tool_invoke` executors, reject direct provider calls to discoverable tools, and keep real execution routed through the existing kernel-governed tool plane.

**Tech Stack:** Rust, existing `loongclaw-app` tool/runtime/provider/conversation modules, `serde`, `serde_json`, `sha2`, `base64`, cargo test, cargo fmt, cargo clippy.

---

### Task 1: Add the discovery-first tool catalog primitives

**Files:**
- Create: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn provider_tool_definitions_only_expose_core_discovery_tools() {
    let defs = provider_tool_definitions();
    let names: Vec<&str> = defs
        .iter()
        .filter_map(|item| item.get("function"))
        .filter_map(|function| function.get("name"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(names, vec!["tool_invoke", "tool_search"]);
}

#[test]
fn provider_exposed_tool_gate_rejects_discoverable_tools() {
    assert!(is_provider_exposed_tool_name("tool.search"));
    assert!(!is_provider_exposed_tool_name("file.read"));
    assert!(!is_provider_exposed_tool_name("shell.exec"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app provider_tool_definitions_only_expose_core_discovery_tools provider_exposed_tool_gate_rejects_discoverable_tools -- --nocapture`
Expected: FAIL because the provider tool schema is still the old full static surface and there is no provider-core exposure gate yet.

**Step 3: Write minimal implementation**

Create a catalog module with deterministic metadata for:

- provider-core tools
  - `tool.search`
  - `tool.invoke`
- discoverable tools
  - `claw.migrate`
  - `external_skills.fetch`
  - `external_skills.inspect`
  - `external_skills.install`
  - `external_skills.invoke`
  - `external_skills.list`
  - `external_skills.policy`
  - `external_skills.remove`
  - feature-gated `file.read`
  - feature-gated `file.write`
  - feature-gated `shell.exec`

Expose helpers such as:

```rust
pub enum ToolExposureClass {
    ProviderCore,
    Discoverable,
}

pub struct ToolCatalogEntry {
    pub canonical_name: &'static str,
    pub provider_function_name: &'static str,
    pub summary: &'static str,
    pub argument_hint: &'static str,
    pub required_fields: &'static [&'static str],
    pub tags: &'static [&'static str],
    pub exposure: ToolExposureClass,
}
```

Refactor `provider_tool_definitions()` to emit only the provider-core pair.
Add `is_provider_exposed_tool_name(...)` and keep `is_known_tool_name(...)`
for internal dispatch use.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app provider_tool_definitions_only_expose_core_discovery_tools provider_exposed_tool_gate_rejects_discoverable_tools -- --nocapture`
Expected: PASS with only the two core tool functions exposed.

**Step 5: Commit**

```bash
git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs
git commit -m "feat: add discovery-first tool catalog"
```

### Task 2: Add `tool_search` with compact cards and leases

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn tool_search_returns_discoverable_tools_with_leases() {
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "read repo file", "limit": 3}),
        },
        &test_config(),
    )
    .expect("tool search should succeed");

    assert_eq!(outcome.status, "ok");
    let results = outcome.payload["results"].as_array().expect("results");
    assert!(!results.is_empty());
    assert!(results.iter().all(|entry| entry["tool_id"] != "tool.search"));
    assert!(results[0]["lease"].as_str().is_some());
}

#[test]
fn tool_search_result_includes_compact_argument_hints() {
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "shell command"}),
        },
        &test_config(),
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().any(|entry| {
        entry["tool_id"] == "shell.exec"
            && entry["argument_hint"].as_str() == Some("command:string,args?:string[]")
    }));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app tool_search_returns_discoverable_tools_with_leases tool_search_result_includes_compact_argument_hints -- --nocapture`
Expected: FAIL because `tool.search` does not exist yet.

**Step 3: Write minimal implementation**

Add a new app-native `tool.search` executor that:

- accepts `query` and optional `limit`
- searches only discoverable catalog entries
- uses deterministic local ranking from:
  - exact name match
  - query token match in summary
  - query token match in tags
  - argument-hint match
- returns:

```rust
json!({
    "query": query,
    "returned": results.len(),
    "results": [{
        "tool_id": entry.canonical_name,
        "summary": entry.summary,
        "argument_hint": entry.argument_hint,
        "required_fields": entry.required_fields,
        "tags": entry.tags,
        "why": why,
        "lease": lease_token,
    }]
})
```

Implement a short-lived signed lease token using:

- JSON claims
- URL-safe base64 encoding
- SHA-256 over `secret || encoded_claims`

Bind the lease to:

- tool id
- expiry
- catalog digest

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tool_search_returns_discoverable_tools_with_leases tool_search_result_includes_compact_argument_hints -- --nocapture`
Expected: PASS with stable search results and non-empty leases.

**Step 5: Commit**

```bash
git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs
git commit -m "feat: add app-native tool search and leases"
```

### Task 3: Add `tool_invoke` and fail closed on invalid leases

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn tool_invoke_dispatches_a_discovered_tool_with_a_valid_lease() {
    let search = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "read file"}),
        },
        &test_config(),
    )
    .expect("tool search should succeed");

    let result = search.payload["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|entry| entry["tool_id"] == "file.read")
        .expect("file.read result");

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "file.read",
                "lease": result["lease"].clone(),
                "arguments": { "path": "README.md", "max_bytes": 64 }
            }),
        },
        &test_config(),
    )
    .expect("tool invoke should succeed");

    assert_eq!(outcome.status, "ok");
}

#[test]
fn tool_invoke_rejects_tampered_or_missing_leases() {
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "file.read",
                "lease": "tampered",
                "arguments": { "path": "README.md" }
            }),
        },
        &test_config(),
    )
    .expect_err("tampered lease should fail");

    assert!(error.contains("invalid_tool_lease"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app tool_invoke_dispatches_a_discovered_tool_with_a_valid_lease tool_invoke_rejects_tampered_or_missing_leases -- --nocapture`
Expected: FAIL because `tool.invoke` does not exist yet.

**Step 3: Write minimal implementation**

Add a `tool.invoke` executor that:

- accepts:
  - `tool_id`
  - `lease`
  - `arguments`
- validates the lease
- rejects:
  - expired lease
  - tampered lease
  - lease/tool mismatch
  - attempts to invoke provider-core tools
- dispatches the underlying discoverable tool by calling a narrow internal
  helper, for example:

```rust
fn execute_discoverable_tool_core_with_config(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String>
```

Do not recurse through `tool.invoke` again. The dispatcher must call only the
real discoverable tool executors.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tool_invoke_dispatches_a_discovered_tool_with_a_valid_lease tool_invoke_rejects_tampered_or_missing_leases -- --nocapture`
Expected: PASS with successful dispatch for a valid lease and fail-closed
behavior for invalid leases.

**Step 5: Commit**

```bash
git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs
git commit -m "feat: add discovery dispatcher for non-core tools"
```

### Task 4: Enforce core-only provider calls in fast-lane and safe-lane

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[tokio::test]
async fn turn_engine_rejects_direct_discoverable_tool_calls() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "file.read".to_owned(),
            args_json: json!({"path": "README.md"}),
            source: "provider_tool_call".to_owned(),
            session_id: String::new(),
            turn_id: String::new(),
            tool_call_id: "call-1".to_owned(),
        }],
        raw_meta: json!({}),
    };

    let result = TurnEngine::new(1).execute_turn(&turn, None).await;
    let failure = result.failure().expect("direct discoverable tool should fail");
    assert_eq!(failure.code, "tool_not_provider_exposed");
}
```

Add an equivalent safe-lane regression that ensures the coordinator's
single-tool execution path also rejects direct discoverable tool names.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app turn_engine_rejects_direct_discoverable_tool_calls -- --nocapture`
Expected: FAIL because provider turns still accept any statically known tool.

**Step 3: Write minimal implementation**

Replace direct provider validation from `is_known_tool_name(...)` to
`is_provider_exposed_tool_name(...)` in:

- `TurnEngine::evaluate_turn(...)`
- `TurnEngine::execute_turn(...)`
- the provider safe-lane single-tool path in `turn_coordinator.rs`

Return a distinct error such as:

```rust
TurnResult::policy_denied(
    "tool_not_provider_exposed",
    format!("tool_not_provider_exposed: {}", intent.tool_name),
)
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app turn_engine_rejects_direct_discoverable_tool_calls -- --nocapture`
Expected: PASS with direct non-core provider calls rejected.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/turn_engine.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "feat: enforce core-only provider tool calls"
```

### Task 5: Shrink prompt exposure and close the permissive fallback

**Files:**
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/tools/external_skills.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Test: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/provider/tests.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn capability_snapshot_only_describes_discovery_runtime_contract() {
    let snapshot = capability_snapshot();
    assert!(snapshot.contains("tool.search"));
    assert!(snapshot.contains("tool.invoke"));
    assert!(!snapshot.contains("file.read"));
    assert!(!snapshot.contains("shell.exec"));
}

#[test]
fn external_skills_auto_expose_default_is_disabled() {
    let config = ToolRuntimeConfig::default();
    assert!(!config.external_skills.auto_expose_installed);
}

#[test]
fn required_capabilities_follow_effective_tool_request() {
    // `tool.invoke(file.read)` must require FilesystemRead.
    // Writeful `claw.migrate` must also require FilesystemWrite.
}

#[tokio::test]
async fn integ_file_read_sandbox_rejects_path_escape_as_policy_denial() {
    // File root escapes must not be treated as retryable execution errors.
}

#[test]
fn from_env_defaults_to_empty_allowlist() {
    crate::process_env::remove_var("LOONGCLAW_SHELL_ALLOWLIST");
    let config = ToolRuntimeConfig::from_env();
    assert!(config.shell_allowlist.is_empty());
}

#[tokio::test]
async fn execute_tool_requires_kernel_context() {
    let error = execute_tool(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "file"}),
        },
        None,
    )
    .await
    .expect_err("missing kernel context should fail");

    assert!(error.contains("kernel_context_required"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app capability_snapshot_only_describes_discovery_runtime_contract external_skills_auto_expose_default_is_disabled execute_tool_requires_kernel_context -- --nocapture`
Expected: FAIL because the prompt snapshot still leaks the full tool surface,
installed skills auto-expose by default, and `execute_tool(..., None)` still
falls through.

**Step 3: Write minimal implementation**

- Replace the full capability snapshot with a compact discovery contract block.
- Update `request_message_runtime.rs` to use the compact snapshot.
- Change `ExternalSkillsRuntimePolicy::default()` and env fallback behavior so
  auto-exposure defaults to `false`.
- Restore default-deny shell behavior when no explicit allowlist is configured.
- Derive required kernel capabilities from the effective `tool.invoke` target.
- Prefix file-root and migration-root escapes with `policy_denied:` so tool
  retries stay fail-closed.
- Change `execute_tool(...)` to fail closed when `kernel_ctx` is `None`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app capability_snapshot_only_describes_discovery_runtime_contract external_skills_auto_expose_default_is_disabled required_capabilities_follow_effective_tool_request integ_file_read_sandbox_rejects_path_escape_as_policy_denial from_env_defaults_to_empty_allowlist execute_tool_requires_kernel_context -- --nocapture`
Expected: PASS with the new compact runtime contract, restored capability
boundaries, fail-closed path denial, default-deny shell behavior, and closed
fallback.

**Step 5: Commit**

```bash
git add crates/app/src/tools/runtime_config.rs crates/app/src/tools/external_skills.rs crates/app/src/tools/mod.rs crates/app/src/provider/request_message_runtime.rs crates/app/src/provider/tests.rs
git commit -m "feat: shrink provider prompt exposure and close tool bypass"
```

### Task 6: Run CI-parity verification and prepare GitHub delivery

**Files:**
- Modify: `docs/plans/2026-03-15-tool-discovery-architecture-design.md`
- Modify: `docs/plans/2026-03-15-tool-discovery-architecture.md`
- Optional: issue/PR metadata only, no code changes unless verification reveals problems

**Step 1: Run focused regression checks**

Run:

```bash
cargo test -p loongclaw-app tool_search_returns_discoverable_tools_with_leases tool_invoke_dispatches_a_discovered_tool_with_a_valid_lease turn_engine_rejects_direct_discoverable_tool_calls -- --nocapture
```

Expected: PASS.

**Step 2: Run formatting**

Run:

```bash
cargo fmt --all -- --check
```

Expected: exit 0.

**Step 3: Run lints**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: exit 0.

**Step 4: Run workspace tests**

Run:

```bash
cargo test --workspace --all-features
```

Expected: exit 0.

**Step 5: Update docs or tests if verification reveals gaps**

Keep changes focused. Do not add unrelated refactors.

**Step 6: Commit final verification fixes**

```bash
git add docs/plans/2026-03-15-tool-discovery-architecture-design.md docs/plans/2026-03-15-tool-discovery-architecture.md
git commit -m "docs: record tool discovery architecture plan"
```
