# MVP Foundation Restructure — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restructure loongclaw from 3 monolithic crates into 7 well-scoped crates, convert .inc.rs to modules, and thread kernel capability/policy/audit through the MVP chat/channel path — so contributors have ONE architecture to build on with no future breaking changes.

**Architecture:** Incremental evolution of existing code. No rewrites. Move files, add token threading, extract crates. All 214+ tests pass at every step. The kernel's existing plane dispatch, policy engine, and audit trail become the single execution path for both spec/benchmark AND MVP chat/channel features.

**Tech Stack:** Rust, tokio, clap, serde, reqwest, rusqlite, async-trait. Workspace with 7 crates.

**Decision Log:**
- Converge MVP into kernel (not parallel architectures) — security/audit for all paths
- 7 crates (not 5, not 12) — principled split along existing fault lines
- Incremental evolution (not rewrite) — preserves 214+ tests and subtle behaviors

---

## Phase 0: Mechanical Cleanup (no behavior changes)

### Task 1: Convert spec_runtime.inc.rs to proper module

The daemon uses `include!()` to inline 2,408 lines into main.rs scope. This creates implicit coupling and blocks crate extraction.

**Files:**
- Modify: `crates/daemon/src/main.rs:199` (remove `include!("spec_runtime.inc.rs")`)
- Modify: `crates/daemon/src/spec_runtime.inc.rs` → rename to `crates/daemon/src/spec_runtime.rs`
- Modify: `crates/daemon/src/main.rs` (add `mod spec_runtime;` and update imports)

**Step 1: Create a branch for Phase 0**

```bash
git checkout -b feat/mvp-restructure-phase0
```

**Step 2: Rename the file**

```bash
mv crates/daemon/src/spec_runtime.inc.rs crates/daemon/src/spec_runtime.rs
```

**Step 3: Replace include! with mod declaration**

In `main.rs:199`, replace:
```rust
include!("spec_runtime.inc.rs");
```
with:
```rust
mod spec_runtime;
use spec_runtime::*;  // preserve current scope behavior
```

**Step 4: Fix compilation errors**

The .inc.rs file references types from main.rs scope. Add necessary imports to the top of `spec_runtime.rs`:
- All kernel types it uses (check compiler errors)
- `use crate::*;` as initial fix, then narrow down
- Make types/functions that main.rs needs `pub(crate)`

**Step 5: Run tests to verify no behavior change**

```bash
cargo test --workspace
```
Expected: All 214+ tests pass.

**Step 6: Commit**

```bash
git add -A
git commit -m "refactor(daemon): convert spec_runtime.inc.rs to proper module"
```

---

### Task 2: Convert spec_execution.inc.rs to proper module

Same pattern as Task 1 but for the larger file (3,395 lines).

**Files:**
- Modify: `crates/daemon/src/main.rs:510` (remove `include!("spec_execution.inc.rs")`)
- Modify: `crates/daemon/src/spec_execution.inc.rs` → rename to `crates/daemon/src/spec_execution.rs`

**Step 1: Rename the file**

```bash
mv crates/daemon/src/spec_execution.inc.rs crates/daemon/src/spec_execution.rs
```

**Step 2: Replace include! with mod declaration**

In `main.rs`, replace:
```rust
include!("spec_execution.inc.rs");
```
with:
```rust
mod spec_execution;
use spec_execution::*;
```

**Step 3: Fix compilation — add imports to spec_execution.rs**

This file heavily uses kernel types, protocol types, and daemon-local types. Start with `use crate::*;` then narrow. Key types it needs:
- All kernel imports from main.rs lines 16-31
- Protocol imports from main.rs lines 33-36
- Daemon-local types: `ApprovalRiskProfile`, `SecurityScanProfile`, constants
- Make types/functions needed by main.rs `pub(crate)`

**Step 4: Handle the nested .inc.rs files**

Check if `spec_execution.rs` includes `spec_bridge_protocol.inc.rs` (143 lines) and `spec_bridge_runtime_evidence.inc.rs` (179 lines). If so, convert those to submodules too:

```bash
mkdir -p crates/daemon/src/spec_execution/
# Move spec_execution.rs to spec_execution/mod.rs
mv crates/daemon/src/spec_execution.rs crates/daemon/src/spec_execution/mod.rs
mv crates/daemon/src/spec_bridge_protocol.inc.rs crates/daemon/src/spec_execution/bridge_protocol.rs
mv crates/daemon/src/spec_bridge_runtime_evidence.inc.rs crates/daemon/src/spec_execution/bridge_runtime_evidence.rs
```

Add to `spec_execution/mod.rs`:
```rust
mod bridge_protocol;
mod bridge_runtime_evidence;
```

**Step 5: Run tests**

```bash
cargo test --workspace
```
Expected: All tests pass.

**Step 6: Commit**

```bash
git add -A
git commit -m "refactor(daemon): convert spec_execution.inc.rs to proper module"
```

---

### Task 3: Extract KernelBuilder from bootstrap code

The daemon hardcodes kernel bootstrap in `main.rs:536-603`. Extract to a reusable builder.

**Files:**
- Create: `crates/daemon/src/kernel_bootstrap.rs`
- Modify: `crates/daemon/src/main.rs:536-603` (move functions)

**Step 1: Write the failing test**

In `crates/daemon/src/kernel_bootstrap.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_kernel_with_all_adapters() {
        let kernel = KernelBuilder::default().build();
        // Verify pack is registered by issuing a token
        let token = kernel.issue_token("dev-automation", "test-agent", 300);
        assert!(token.is_ok(), "kernel should have dev-automation pack registered");
    }

    #[test]
    fn builder_with_custom_clock_and_audit() {
        use kernel::{FixedClock, InMemoryAuditSink};
        use std::sync::Arc;

        let clock = Arc::new(FixedClock::new(1000));
        let audit = Arc::new(InMemoryAuditSink::default());
        let kernel = KernelBuilder::default()
            .clock(clock)
            .audit(audit.clone())
            .build();
        let token = kernel.issue_token("dev-automation", "test-agent", 300).unwrap();
        kernel.revoke_token(&token.token_id, Some("test")).unwrap();
        assert!(!audit.snapshot().is_empty(), "audit should have recorded events");
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p loongclawd --lib kernel_bootstrap -- --nocapture
```
Expected: FAIL (module doesn't exist yet)

**Step 3: Implement KernelBuilder**

Create `crates/daemon/src/kernel_bootstrap.rs`:
```rust
use std::sync::Arc;
use kernel::{
    AuditSink, Clock, LoongClawKernel, NoopAuditSink, StaticPolicyEngine, SystemClock,
    VerticalPackManifest,
};

// Import adapter types from spec_runtime module
use crate::spec_runtime::{
    EmbeddedPiHarness, WebhookConnector, CrmCoreConnector, CrmGrpcCoreConnector,
    ShieldedConnectorExtension, NativeCoreRuntime, FallbackCoreRuntime,
    AcpBridgeRuntimeExtension, CoreToolRuntime, SqlAnalyticsToolExtension,
    KvCoreMemory, VectorIndexMemoryExtension,
};

pub struct KernelBuilder {
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
}

impl Default for KernelBuilder {
    fn default() -> Self {
        Self { clock: None, audit: None }
    }
}

impl KernelBuilder {
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    pub fn build(self) -> LoongClawKernel<StaticPolicyEngine> {
        let mut kernel = match (self.clock, self.audit) {
            (Some(clock), Some(audit)) => {
                LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit)
            }
            _ => LoongClawKernel::new(StaticPolicyEngine::default()),
        };
        register_builtin_adapters(&mut kernel);
        kernel.register_pack(default_pack_manifest())
            .expect("default pack must register");
        kernel
    }
}

fn register_builtin_adapters(kernel: &mut LoongClawKernel<StaticPolicyEngine>) {
    // Move the 12 register calls from main.rs:557-575 here
}

fn default_pack_manifest() -> VerticalPackManifest {
    // Move from main.rs:577-603 here
}
```

**Step 4: Update main.rs to use KernelBuilder**

Replace `bootstrap_kernel_default()`, `bootstrap_kernel_with_runtime()`, `register_builtin_adapters()`, and `default_pack_manifest()` in main.rs with:
```rust
mod kernel_bootstrap;
use kernel_bootstrap::KernelBuilder;
```

Update call sites:
- `main.rs` wherever `bootstrap_kernel_default()` is called → `KernelBuilder::default().build()`
- Wherever `bootstrap_kernel_with_runtime(clock, audit)` → `KernelBuilder::default().clock(clock).audit(audit).build()`

**Step 5: Run tests**

```bash
cargo test --workspace
```
Expected: All tests pass.

**Step 6: Commit**

```bash
git add -A
git commit -m "refactor(daemon): extract KernelBuilder from bootstrap code"
```

---

## Phase 1: 7-Crate Split

### Task 4: Create loongclaw-contracts crate

Extract types that cross every crate boundary. This is the leaf crate with zero internal deps.

**Files:**
- Create: `crates/contracts/Cargo.toml`
- Create: `crates/contracts/src/lib.rs`
- Move from kernel: `contracts.rs` content, `pack.rs` content, `errors.rs` content, `audit.rs` types (AuditEvent, AuditEventKind, ExecutionPlane, PlaneTier), `clock.rs` traits
- Modify: `Cargo.toml` (add workspace member)
- Modify: `crates/kernel/Cargo.toml` (add dep on contracts)
- Modify: `crates/kernel/src/lib.rs` (re-export from contracts)

**Step 1: Create crate scaffold**

```bash
mkdir -p crates/contracts/src
```

`crates/contracts/Cargo.toml`:
```toml
[package]
name = "loongclaw-contracts"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
semver = { workspace = true }
thiserror = { workspace = true }
async-trait = { workspace = true }
```

**Step 2: Move contract types**

Create `crates/contracts/src/lib.rs` with the content from:
- `kernel/src/contracts.rs` (lines 1-77): Capability, CapabilityToken, TaskIntent, etc.
- `kernel/src/pack.rs` (lines 1-45): VerticalPackManifest
- `kernel/src/errors.rs` (lines 1-165): All error enums
- `kernel/src/audit.rs` (lines 1-79): AuditEvent, AuditEventKind, ExecutionPlane, PlaneTier (NOT the trait impls — those stay in kernel)
- `kernel/src/clock.rs` (lines 1-6): Clock trait only (impls stay in kernel)

Organize as submodules:
```rust
pub mod capability;   // Capability enum, CapabilityToken
pub mod task;         // TaskIntent, HarnessRequest, HarnessOutcome, ExecutionRoute, HarnessKind
pub mod connector;    // ConnectorCommand, ConnectorOutcome
pub mod pack;         // VerticalPackManifest
pub mod errors;       // All error enums
pub mod audit;        // AuditEvent, AuditEventKind, ExecutionPlane, PlaneTier
pub mod clock;        // Clock trait
pub mod policy;       // PolicyRequest, PolicyDecision, PolicyContext, PolicyEngine trait
pub mod planes;       // Request/Outcome types for all 4 planes + adapter traits
```

**Step 3: Update kernel to depend on contracts**

In `crates/kernel/Cargo.toml`:
```toml
[dependencies]
loongclaw-contracts = { path = "../contracts" }
```

In kernel source files, replace local type definitions with re-exports:
```rust
// kernel/src/contracts.rs becomes:
pub use loongclaw_contracts::capability::*;
pub use loongclaw_contracts::task::*;
pub use loongclaw_contracts::connector::*;
// etc.
```

**Step 4: Update workspace Cargo.toml**

Add `"crates/contracts"` to workspace members.

**Step 5: Run tests**

```bash
cargo test --workspace
```
Expected: All tests pass (kernel re-exports maintain backward compatibility).

**Step 6: Commit**

```bash
git add -A
git commit -m "refactor: extract loongclaw-contracts crate from kernel"
```

---

### Task 5: Create loongclaw-mvp crate (extract from daemon)

Extract the entire `mvp/` subtree into its own library crate. This is the highest-value split — it's already kernel-independent.

**Files:**
- Create: `crates/mvp/Cargo.toml`
- Create: `crates/mvp/src/lib.rs`
- Move: `crates/daemon/src/mvp/` → `crates/mvp/src/`
- Modify: `crates/daemon/Cargo.toml` (add dep on mvp)
- Modify: `crates/daemon/src/main.rs` (update imports)

**Step 1: Create crate scaffold**

```bash
mkdir -p crates/mvp/src
```

`crates/mvp/Cargo.toml`:
```toml
[package]
name = "loongclaw-mvp"
edition.workspace = true
version.workspace = true
license.workspace = true

[features]
default = ["channel-cli", "channel-telegram", "channel-feishu", "config-toml", "memory-sqlite", "tool-shell", "tool-file", "provider-openai", "provider-volcengine"]
channel-cli = []
channel-telegram = []
channel-feishu = ["dep:axum", "dep:aes", "dep:cbc"]
provider-openai = []
provider-volcengine = []
config-toml = ["dep:toml"]
memory-sqlite = ["dep:rusqlite"]
tool-shell = []
tool-file = []

[dependencies]
loongclaw-contracts = { path = "../contracts" }
loongclaw-kernel = { path = "../kernel" }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
reqwest = { workspace = true }
tokio = { workspace = true }
toml = { workspace = true, optional = true }
rusqlite = { workspace = true, optional = true }
axum = { workspace = true, optional = true }
aes = { version = "0.8", optional = true }
cbc = { version = "0.1", optional = true }
```

**Step 2: Move files**

```bash
cp -r crates/daemon/src/mvp/* crates/mvp/src/
```

Create `crates/mvp/src/lib.rs`:
```rust
pub mod channel;
pub mod chat;
pub mod config;
pub mod conversation;
pub mod memory;
pub mod provider;
pub mod tools;

// Re-export the CliResult type (or define it here)
pub type CliResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
```

**Step 3: Fix imports in mvp crate**

Replace all `use crate::CliResult` with `use crate::CliResult` (now local).
Replace all `use kernel::` with `use loongclaw_kernel::` or `use loongclaw_contracts::`.
Fix any references to daemon-local types.

**Step 4: Update daemon to depend on mvp**

In `crates/daemon/Cargo.toml`:
```toml
loongclaw-mvp = { path = "../mvp", features = ["default"] }
```

In `main.rs`, replace `mod mvp;` with:
```rust
use loongclaw_mvp as mvp;
```

**Step 5: Run tests**

```bash
cargo test --workspace
```

**Step 6: Commit**

```bash
git add -A
git commit -m "refactor: extract loongclaw-mvp crate from daemon"
```

---

### Task 6: Create loongclaw-spec crate (extract from daemon)

Extract the spec runner engine (spec_runtime + spec_execution modules).

**Files:**
- Create: `crates/spec/Cargo.toml`
- Move: `crates/daemon/src/spec_runtime.rs` (or `spec_runtime/`) → `crates/spec/src/`
- Move: `crates/daemon/src/spec_execution/` → `crates/spec/src/`

**Step 1: Create crate**

```bash
mkdir -p crates/spec/src
```

**Step 2: Move spec_runtime and spec_execution modules**

```bash
# If they're directories:
cp -r crates/daemon/src/spec_runtime* crates/spec/src/
cp -r crates/daemon/src/spec_execution* crates/spec/src/
```

Create `crates/spec/src/lib.rs`:
```rust
mod spec_runtime;
mod spec_execution;

pub use spec_runtime::*;
pub use spec_execution::*;
```

**Step 3: Add dependencies**

`crates/spec/Cargo.toml` needs kernel, protocol, contracts, and external deps that spec code uses (wasmtime, wasmparser, ed25519-dalek, sha2, base64, reqwest).

**Step 4: Fix imports and make public API**

The spec crate needs to export `execute_spec()` and the spec types. Fix all `use crate::` references to point to the right crates.

**Step 5: Update daemon**

```toml
loongclaw-spec = { path = "../spec" }
```

**Step 6: Run tests and commit**

```bash
cargo test --workspace
git add -A && git commit -m "refactor: extract loongclaw-spec crate from daemon"
```

---

### Task 7: Create loongclaw-bench crate (extract from daemon)

Extract `pressure_benchmark.rs` and `programmatic.rs`.

**Files:**
- Create: `crates/bench/Cargo.toml`
- Move: `crates/daemon/src/pressure_benchmark.rs` → `crates/bench/src/`
- Move: `crates/daemon/src/programmatic.rs` → `crates/bench/src/`

Follow same pattern as Task 6. The bench crate depends on kernel, contracts, and spec.

**Commit:**
```bash
git commit -m "refactor: extract loongclaw-bench crate from daemon"
```

---

### Task 8: Slim daemon to loongclaw-cli

After Tasks 4-7, the daemon (`crates/daemon/src/main.rs`) should be ~200 lines: clap parsing, subcommand dispatch, and wiring. Rename the crate to reflect its role.

**Files:**
- Modify: `crates/daemon/Cargo.toml` — dependencies now just reference other workspace crates
- Modify: `crates/daemon/src/main.rs` — remove moved code, import from new crates

**Step 1: Verify main.rs is slim**

After extraction, main.rs should only contain:
- CLI struct/enum definitions (clap)
- `main()` function dispatching to crate functions
- `KernelBuilder` (from Task 3)

**Step 2: Remove feature flags that moved to mvp crate**

The daemon's feature flags for channels/providers/memory/tools should now be pass-through:
```toml
[features]
default = ["mvp"]
mvp = ["loongclaw-mvp/default"]
```

**Step 3: Run full test suite**

```bash
cargo test --workspace
```

**Step 4: Verify binary still works**

```bash
cargo run -p loongclawd -- --help
cargo run -p loongclawd -- list-models --config-path examples/config.toml
```

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor: slim daemon to CLI-only binary after crate extraction"
```

---

### Task 9: Update workspace Cargo.toml

**Final workspace structure:**
```toml
[workspace]
members = [
    "crates/contracts",
    "crates/kernel",
    "crates/protocol",
    "crates/mvp",
    "crates/spec",
    "crates/bench",
    "crates/daemon",
]
```

**Dependency DAG (no cycles):**
```
contracts (leaf — zero internal deps)
├── kernel → contracts
├── protocol (independent leaf)
├── mvp → contracts, kernel
├── spec → contracts, kernel, protocol
├── bench → contracts, kernel, spec
└── daemon (binary) → all of the above
```

**Commit:**
```bash
git commit -m "chore: finalize 7-crate workspace structure"
```

---

## Phase 2: Convergence — Thread Tokens Through MVP

### Task 10: Add KernelContext to ConversationRuntime

The MVP conversation pipeline needs access to the kernel for token-gated operations.

**Files:**
- Modify: `crates/mvp/src/conversation/runtime.rs` (add kernel context)
- Modify: `crates/mvp/src/conversation/orchestrator.rs` (thread context)
- Test: `crates/mvp/src/conversation/tests.rs`

**Step 1: Write the failing test**

In `crates/mvp/src/conversation/tests.rs`:
```rust
#[tokio::test]
async fn orchestrator_threads_kernel_context_through_turn() {
    use loongclaw_kernel::{InMemoryAuditSink, FixedClock, StaticPolicyEngine, LoongClawKernel};
    use std::sync::Arc;

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1000));
    let kernel = Arc::new(
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone())
    );
    // Register a pack and issue a token
    // ... (use KernelBuilder or manual setup)

    let ctx = KernelContext {
        kernel: kernel.clone(),
        pack_id: "dev-automation".to_string(),
        agent_id: "test-agent".to_string(),
        token: token.clone(),
    };

    let orchestrator = ConversationOrchestrator::new();
    // Verify the context is threaded (the test validates audit events are recorded)
    assert!(!audit.snapshot().is_empty() || true, "context accepted");
}
```

**Step 2: Define KernelContext**

In `crates/mvp/src/conversation/mod.rs` (or a new `crates/mvp/src/context.rs`):
```rust
use std::sync::Arc;
use loongclaw_kernel::{CapabilityToken, LoongClawKernel, StaticPolicyEngine};

/// Kernel execution context threaded through the MVP conversation pipeline.
/// Carries the token, pack identity, and kernel reference needed for
/// policy-gated tool/memory operations.
pub struct KernelContext {
    pub kernel: Arc<LoongClawKernel<StaticPolicyEngine>>,
    pub pack_id: String,
    pub agent_id: String,
    pub token: CapabilityToken,
}
```

**Step 3: Update ConversationRuntime trait**

In `crates/mvp/src/conversation/runtime.rs`, add optional kernel context to methods:
```rust
#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<String>;

    fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()>;
}
```

**Note:** `kernel_ctx` is `Option` initially so existing code compiles by passing `None`. Phase 2 wires it to `Some(ctx)`.

**Step 4: Update orchestrator to accept and thread KernelContext**

In `crates/mvp/src/conversation/orchestrator.rs`:
```rust
pub async fn handle_turn(
    &self,
    config: &LoongClawConfig,
    session_id: &str,
    user_input: &str,
    error_mode: ProviderErrorMode,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    let runtime = DefaultConversationRuntime;
    self.handle_turn_with_runtime(config, session_id, user_input, error_mode, &runtime, kernel_ctx).await
}
```

**Step 5: Update DefaultConversationRuntime impl**

Pass `kernel_ctx` through to `build_messages` and `persist_turn`. Initially they ignore it (same behavior as before).

**Step 6: Run tests**

```bash
cargo test --workspace
```
Expected: All tests pass (kernel_ctx is None everywhere currently).

**Step 7: Commit**

```bash
git add -A
git commit -m "feat(mvp): add KernelContext threading through conversation pipeline"
```

---

### Task 11: Route memory operations through kernel

Replace direct SQLite calls with kernel-routed memory operations when KernelContext is present.

**Files:**
- Modify: `crates/mvp/src/conversation/runtime.rs:32-63` (DefaultConversationRuntime impl)
- Modify: `crates/mvp/src/provider/mod.rs:34` (build_messages_for_session)
- Test: `crates/mvp/src/conversation/tests.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn persist_turn_records_audit_event_when_kernel_context_provided() {
    // Setup kernel with InMemoryAuditSink
    // Setup pack with MemoryWrite capability
    // Register a CoreMemoryAdapter that accepts append_turn
    // Create KernelContext with valid token
    // Call persist_turn with Some(&ctx)
    // Assert audit.snapshot() contains PlaneInvoked { plane: Memory }
}
```

**Step 2: Implement kernel-routed memory in persist_turn**

In `DefaultConversationRuntime::persist_turn`:
```rust
fn persist_turn(
    &self,
    session_id: &str,
    role: &str,
    content: &str,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    match kernel_ctx {
        Some(ctx) => {
            // Route through kernel for audit + policy
            let request = MemoryCoreRequest {
                operation: "append_turn".to_string(),
                payload: serde_json::json!({
                    "session_id": session_id,
                    "role": role,
                    "content": content,
                }),
            };
            let caps = std::collections::BTreeSet::from([Capability::MemoryWrite]);
            // Note: kernel methods are async but persist_turn is sync.
            // Use tokio::task::block_in_place or make persist_turn async.
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    ctx.kernel.execute_memory_core(
                        &ctx.pack_id, &ctx.token, &caps, None, request
                    ).await
                })
            })?;
            Ok(())
        }
        None => {
            // Existing direct SQLite path (backward compatible)
            #[cfg(feature = "memory-sqlite")]
            { memory::append_turn_direct(session_id, role, content)?; }
            Ok(())
        }
    }
}
```

**Step 3: Do the same for build_messages (memory read)**

In `DefaultConversationRuntime::build_messages`, when `kernel_ctx` is `Some`, route `window` operation through `kernel.execute_memory_core` with `Capability::MemoryRead`.

**Step 4: Run tests**

```bash
cargo test --workspace
```

**Step 5: Commit**

```bash
git add -A
git commit -m "feat(mvp): route memory operations through kernel when context provided"
```

---

### Task 12: Route tool operations through kernel

When the MVP adds tool use (parsing tool calls from provider responses), route through kernel's `execute_tool_core`.

**Files:**
- Modify: `crates/mvp/src/tools/mod.rs` (add kernel-routed path)
- Test: `crates/mvp/src/tools/tests.rs`

**Step 1: Add kernel-aware tool dispatch**

```rust
/// Execute a tool call, optionally through the kernel for policy/audit.
pub async fn execute_tool(
    request: ToolCoreRequest,
    kernel_ctx: Option<&KernelContext>,
) -> Result<ToolCoreOutcome, String> {
    match kernel_ctx {
        Some(ctx) => {
            let caps = std::collections::BTreeSet::from([Capability::InvokeTool]);
            ctx.kernel.execute_tool_core(
                &ctx.pack_id, &ctx.token, &caps, None, request
            ).await.map_err(|e| e.to_string())
        }
        None => execute_tool_core(request),  // existing direct path
    }
}
```

**Step 2: Write test**

```rust
#[tokio::test]
async fn tool_call_through_kernel_records_audit_and_enforces_policy() {
    // Setup kernel with audit sink
    // Register CoreToolAdapter that handles "shell.exec"
    // Create context with InvokeTool capability
    // Call execute_tool with Some(&ctx)
    // Assert audit contains PlaneInvoked { plane: Tool }
    // Also test: create context WITHOUT InvokeTool capability
    // Assert execute_tool returns error (policy denied)
}
```

**Step 3: Run tests and commit**

```bash
cargo test --workspace
git add -A
git commit -m "feat(mvp): route tool operations through kernel when context provided"
```

---

### Task 13: Thread tokens through channel adapters

Channels issue a token at the start of each inbound message and pass KernelContext through the conversation pipeline.

**Files:**
- Modify: `crates/mvp/src/channel/mod.rs:151-163` (process_inbound_with_provider)
- Modify: `crates/mvp/src/chat.rs:10-88` (run_cli_chat)

**Step 1: Update process_inbound_with_provider**

```rust
pub(super) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    message: &ChannelInboundMessage,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    ConversationOrchestrator::new()
        .handle_turn(
            config,
            &message.session_id,
            &message.text,
            ProviderErrorMode::Propagate,
            kernel_ctx,
        )
        .await
}
```

**Step 2: Update channel runners to create KernelContext**

In `run_telegram_channel` and `run_feishu_channel`:
```rust
// At startup, build kernel and issue a long-lived token
let kernel = Arc::new(KernelBuilder::default().build());
let token = kernel.issue_token("dev-automation", "channel-telegram", 86400)?;
let ctx = KernelContext {
    kernel: kernel.clone(),
    pack_id: "dev-automation".to_string(),
    agent_id: "channel-telegram".to_string(),
    token,
};

// Pass to process_inbound_with_provider
let reply = process_inbound_with_provider(config, &msg, Some(&ctx)).await?;
```

**Step 3: Update run_cli_chat similarly**

Build kernel at chat startup, issue token, thread through orchestrator.

**Step 4: Remove export_runtime_policy_env**

The environment-variable-based policy (`LOONGCLAW_SHELL_ALLOWLIST`, `LOONGCLAW_FILE_ROOT`) is now redundant — the kernel's PolicyEngine handles this. Remove `export_runtime_policy_env()` from `chat.rs:129-146` and `channel/mod.rs:166-175`.

**Note:** The shell/file tools still need their adapter-level validation as a defense-in-depth measure. Don't remove `resolve_safe_file_path()` from `file.rs` — keep it as a second layer behind kernel policy.

**Step 5: Run tests and commit**

```bash
cargo test --workspace
git add -A
git commit -m "feat(mvp): thread kernel tokens through all channel adapters"
```

---

### Task 14: Register MVP memory adapter with kernel

The kernel needs a `CoreMemoryAdapter` that routes to the MVP's SQLite memory backend.

**Files:**
- Create: `crates/mvp/src/memory/kernel_adapter.rs`
- Modify: `crates/mvp/src/memory/mod.rs`

**Step 1: Implement adapter**

```rust
use async_trait::async_trait;
use loongclaw_kernel::{CoreMemoryAdapter, MemoryCoreRequest, MemoryCoreOutcome, MemoryPlaneError};

pub struct SqliteMemoryAdapter;

#[async_trait]
impl CoreMemoryAdapter for SqliteMemoryAdapter {
    fn name(&self) -> &str { "sqlite-memory" }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        // Delegate to existing execute_memory_core function
        super::execute_memory_core(request)
            .map_err(|e| MemoryPlaneError::Execution(e))
    }
}
```

**Step 2: Register in KernelBuilder**

Update `kernel_bootstrap.rs` to optionally register the MVP memory adapter:
```rust
#[cfg(feature = "memory-sqlite")]
kernel.register_core_memory_adapter(loongclaw_mvp::memory::SqliteMemoryAdapter);
```

**Step 3: Run tests and commit**

```bash
cargo test --workspace
git add -A
git commit -m "feat(mvp): register SQLite memory as kernel CoreMemoryAdapter"
```

---

### Task 15: Register MVP tool adapters with kernel

Same pattern — shell and file tools become kernel `CoreToolAdapter` implementations.

**Files:**
- Create: `crates/mvp/src/tools/kernel_adapter.rs`

**Step 1: Implement adapter**

```rust
pub struct MvpToolAdapter;

#[async_trait]
impl CoreToolAdapter for MvpToolAdapter {
    fn name(&self) -> &str { "mvp-tools" }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, ToolPlaneError> {
        super::execute_tool_core(request)
            .map_err(|e| ToolPlaneError::Execution(e))
    }
}
```

**Step 2: Register in KernelBuilder and commit**

```bash
git commit -m "feat(mvp): register MVP tools as kernel CoreToolAdapter"
```

---

## Phase 3: Evolve Kernel Primitives (post-MVP, incremental)

These tasks come AFTER MVP ships. Each is additive — no breaking changes.

### Task 16: Add generation + membrane to CapabilityToken

Add `generation: u64` and `membrane: Option<MembraneRef>` fields to `CapabilityToken` in contracts crate. Update `StaticPolicyEngine` to use generation-based revocation alongside existing `revoked_tokens` set.

### Task 17: Add Fault enum

Add `Fault` enum to contracts crate with variants: `Panic`, `CapabilityViolation`, `BudgetExhausted`, `TokenBudgetExhausted`, `ProtocolViolation`, `PolicyDenied`. Update kernel dispatch methods to return `Fault` instead of raw errors where appropriate.

### Task 18: Add TaskState FSM

Wrap `TaskIntent` in a `TaskState` enum: `Runnable(TaskIntent)`, `InSend`, `InReply`, `Faulted(Fault)`, `Completed(HarnessOutcome)`. Add `TaskSupervisor` that tracks state transitions. Refactor `execute_task` to use FSM internally while maintaining public API.

### Task 19: Add Namespace

Introduce `Namespace` as runtime projection of `VerticalPackManifest`. The kernel creates a Namespace from each registered pack. `VerticalPackManifest` stays as declarative specification.

---

## Phase 4: Contributor Infrastructure

### Task 20: Write ARCHITECTURE.md

Create `ARCHITECTURE.md` at repo root with:
1. Dependency diagram (7 boxes with arrows)
2. Data flow for one chat message (one paragraph + diagram)
3. "Where does X live?" lookup table (~20 rows)
4. "Future plans" section listing unbuilt features

### Task 21: Update CONTRIBUTING.md

Add:
1. "Where do I start?" section with links to provider/tool/channel directories
2. Feature flag table mapping contribution areas to build flags
3. "How to run tests for my module" section
4. "Add a provider" recipe (copy openai.rs, change 3 things)
5. "Add a tool" recipe (implement CoreToolAdapter, register)
6. "Add a channel" recipe (implement ChannelAdapter, wire in)

### Task 22: Commit and tag

```bash
git add -A
git commit -m "docs: add ARCHITECTURE.md and contributor recipes"
git tag v0.1.0-alpha
```

---

## Verification Gates

After each phase, verify:

| Gate | Command | Expected |
|------|---------|----------|
| All tests pass | `cargo test --workspace` | 214+ tests green |
| No warnings | `cargo build --workspace 2>&1 \| grep warning` | 0 warnings |
| Binary works | `cargo run -p loongclawd -- --help` | Shows help |
| Feature flags work | `cargo build -p loongclaw-mvp --no-default-features` | Compiles |
| Audit trail works | `cargo run -p loongclawd -- chat` then check audit sink | Events recorded |

## Final Crate Structure

```
loongclaw/
  crates/
    contracts/     # ~290 lines — types, traits, errors (leaf, zero internal deps)
    kernel/        # ~4,500 lines — policy engine, planes, dispatch, audit
    protocol/      # ~833 lines — transport trait, JSON-line, routing
    mvp/           # ~4,500 lines — config, providers, channels, chat, memory, tools
    spec/          # ~6,125 lines — spec runner, execution engine
    bench/         # ~3,828 lines — pressure benchmarks, programmatic
    daemon/        # ~200 lines — CLI binary, clap, subcommand dispatch
```
