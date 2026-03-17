# Test Convention Overhaul Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Standardize all 7 workspace crates on hybrid test convention: inline unit tests in `src/`, external integration tests in `crates/<name>/tests/`, with `feature = "test-support"` gating internal re-exports.

**Architecture:** Single monolithic PR. Migrate crates bottom-up by dependency order. Each crate gets a `test-support` feature (if needed) and a `tests/` directory for integration tests. Daemon requires lib/bin split since it's currently binary-only.

**Tech Stack:** Rust 2024 edition, tokio async, proptest (kernel), tempfile (app)

**Design doc:** `docs/plans/2026-03-15-test-convention-overhaul-design.md`

---

## Pre-Flight

### Task 0: Create feature branch and verify baseline

**Files:**
- None modified yet

**Step 1: Create branch**
```bash
cd /Users/xiangjun/github/loongclaw/loongclaw
git checkout -b issue-139-test-convention-overhaul
```

**Step 2: Verify baseline passes**
```bash
cargo test --workspace
```
Expected: All tests pass

**Step 3: Commit the design doc**
```bash
git add docs/plans/2026-03-15-test-convention-overhaul-design.md
git add docs/plans/2026-03-15-test-convention-overhaul-implementation-plan.md
git commit -m "docs: add test convention overhaul design and implementation plan

Closes #139"
```

---

## Phase 1: Leaf Crates (Low Risk)

### Task 1: contracts — audit and confirm no migration needed

**Files:**
- Read: `crates/contracts/src/lib.rs`

**Step 1: Verify no tests exist**
```bash
grep -r '#\[cfg(test)\]' crates/contracts/src/
grep -r '#\[test\]' crates/contracts/src/
```
Expected: No matches (0 tests, pure type definitions)

**Step 2: No action needed — skip**

---

### Task 2: bench — audit and confirm no migration needed

**Files:**
- Read: `crates/bench/src/lib.rs`

**Step 1: Verify no tests exist**
```bash
grep -r '#\[test\]' crates/bench/src/
```
Expected: No matches

**Step 2: No action needed — skip**

---

### Task 3: protocol — migrate 21 tests to external `tests/`

**Files:**
- Modify: `crates/protocol/src/lib.rs` — remove inline test module, add test-support feature gate
- Create: `crates/protocol/src/test_support.rs` — test helper re-exports
- Create: `crates/protocol/tests/protocol_transport.rs` — relocated integration tests
- Modify: `crates/protocol/Cargo.toml` — add test-support feature

**Step 1: Add `test-support` feature to Cargo.toml**

In `crates/protocol/Cargo.toml`, add:
```toml
[features]
test-support = []
```

And in `[dev-dependencies]` section, add self-reference:
```toml
[dev-dependencies]
loongclaw-protocol = { path = ".", features = ["test-support"] }
```

**Step 2: Create `src/test_support.rs`**

Extract the test helper function from the inline test module:
```rust
//! Test support utilities for protocol crate.
//! Gated behind `feature = "test-support"`.

use crate::TransportInfo;

/// Create a test TransportInfo with the given name.
pub fn test_transport_info(name: &str) -> TransportInfo {
    // Copy the existing helper implementation from lib.rs tests
    TransportInfo {
        // ... exact fields from current test helper
    }
}
```

**Step 3: Add feature gate in `lib.rs`**

Add near the top of `lib.rs`:
```rust
#[cfg(feature = "test-support")]
pub mod test_support;
```

**Step 4: Create `crates/protocol/tests/protocol_transport.rs`**

Move all 21 tests from the inline `#[cfg(test)] mod tests` block in `lib.rs` to this new file. Update imports:
```rust
use loongclaw_protocol::*;
use loongclaw_protocol::test_support::test_transport_info;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, duplex, split};
use tokio::time::{sleep, timeout};

// ... all 21 test functions, unchanged except import paths
```

**Step 5: Remove inline test module from `lib.rs`**

Delete the entire `#[cfg(test)] mod tests { ... }` block from `lib.rs`.

**Step 6: Run tests**
```bash
cargo test -p loongclaw-protocol
```
Expected: All 21 tests pass

**Step 7: Verify no test-support leak**
```bash
cargo test -p loongclaw-protocol --no-default-features
```
Expected: Pass (test_support not compiled in release)

**Step 8: Commit**
```bash
git add crates/protocol/
git commit -m "refactor(protocol): migrate tests to external tests/ directory

- Add test-support feature with test_transport_info helper
- Move 21 tests from inline mod to crates/protocol/tests/
- Remove inline #[cfg(test)] module from lib.rs"
```

---

### Task 4: spec — migrate 2 tests to external `tests/`

**Files:**
- Modify: `crates/spec/src/lib.rs` — remove inline test module
- Modify: `crates/spec/Cargo.toml` — add test-support feature (extend existing test-hooks)
- Create: `crates/spec/src/test_support.rs` — test helper
- Create: `crates/spec/tests/spec_execution.rs` — relocated tests

**Step 1: Add `test-support` feature to Cargo.toml**

In `crates/spec/Cargo.toml`, update features:
```toml
[features]
test-hooks = []
test-support = ["test-hooks"]
```

And add self-reference in dev-dependencies:
```toml
[dev-dependencies]
loongclaw-spec = { path = ".", features = ["test-support"] }
```

**Step 2: Create `src/test_support.rs`**

```rust
//! Test support utilities for spec crate.
//! Gated behind `feature = "test-support"`.

use crate::OperationSpec;
use crate::RunnerSpec;

/// Create a RunnerSpec from an OperationSpec for testing.
pub fn make_runner_spec(operation: OperationSpec) -> RunnerSpec {
    // Copy exact implementation from current inline helper
}
```

**Step 3: Add feature gate in `lib.rs`**

```rust
#[cfg(feature = "test-support")]
pub mod test_support;
```

**Step 4: Create `crates/spec/tests/spec_execution.rs`**

```rust
use loongclaw_spec::*;
use loongclaw_spec::test_support::make_runner_spec;
use std::collections::{BTreeMap, BTreeSet};
use loongclaw_kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};
use serde_json::json;

// ... 2 test functions relocated
```

**Step 5: Remove inline test module from `lib.rs`**

**Step 6: Run tests**
```bash
cargo test -p loongclaw-spec
```
Expected: 2 tests pass

**Step 7: Commit**
```bash
git add crates/spec/
git commit -m "refactor(spec): migrate tests to external tests/ directory

- Add test-support feature extending test-hooks
- Move 2 tests to crates/spec/tests/
- Extract make_runner_spec helper to test_support module"
```

---

## Phase 2: Core Runtime (Medium Complexity)

### Task 5: kernel — extract mock adapters and migrate integration tests

**Files:**
- Modify: `crates/kernel/src/lib.rs` — add test-support feature gate
- Create: `crates/kernel/src/test_support.rs` — mock adapters and helpers
- Create: `crates/kernel/tests/kernel_integration.rs` — integration tests
- Modify: `crates/kernel/src/tests.rs` — keep only unit tests, update imports
- Modify: `crates/kernel/Cargo.toml` — add test-support feature

**Step 1: Add `test-support` feature to Cargo.toml**

```toml
[features]
test-support = []

[dev-dependencies]
loongclaw-kernel = { path = ".", features = ["test-support"] }
```

**Step 2: Create `src/test_support.rs`**

Extract all mock adapter structs from `src/tests.rs` (lines 40-266):
```rust
//! Test support utilities for kernel crate.
//! Gated behind `feature = "test-support"`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use serde_json::Value;

// Re-export internal types needed by integration tests
pub use crate::{
    FixedClock, InMemoryAuditSink, LoongClawKernel,
    StaticPolicyEngine, VerticalPackManifest,
};

/// Mock embedded Pi harness for testing.
pub struct MockEmbeddedPiHarness { /* ... copy from tests.rs ... */ }

/// Mock CRM connector for testing.
pub struct MockCrmConnector { /* ... copy from tests.rs ... */ }

/// Mock core connector for testing.
pub struct MockCoreConnector { /* ... copy from tests.rs ... */ }

/// Mock memory adapter for testing.
pub struct MockMemoryAdapter { /* ... copy from tests.rs ... */ }

/// Mock runtime adapter for testing.
pub struct MockRuntimeAdapter { /* ... copy from tests.rs ... */ }

/// Mock tool adapter for testing.
pub struct MockToolAdapter { /* ... copy from tests.rs ... */ }

// All trait impls for above structs...

/// Build a sample VerticalPackManifest for testing.
pub fn sample_pack() -> VerticalPackManifest { /* ... */ }

/// Build a capability set from a bitmask.
pub fn capability_set_from_mask(mask: u16) -> BTreeSet<Capability> { /* ... */ }
```

**Step 3: Add feature gate in `lib.rs`**

```rust
#[cfg(feature = "test-support")]
pub mod test_support;
```

**Step 4: Classify tests in `src/tests.rs`**

Read each test and classify:
- **Integration** (full kernel bootstrap, multi-adapter composition): move to `tests/kernel_integration.rs`
- **Unit** (single policy check, single adapter, proptest): keep in `src/tests.rs`

Heuristic: if it calls `LoongClawKernel::with_runtime()` or bootstraps a full kernel, it's integration.

**Step 5: Create `crates/kernel/tests/kernel_integration.rs`**

```rust
use loongclaw_kernel::test_support::*;
use loongclaw_kernel::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use serde_json::json;

// ... integration test functions relocated from src/tests.rs
```

**Step 6: Update `src/tests.rs`**

Remove relocated integration tests and mock adapter definitions. Update remaining unit tests to import from `test_support`:
```rust
#[cfg(test)]
mod tests {
    use crate::test_support::*;
    // ... remaining unit tests
}
```

Wait — unit tests in `src/tests.rs` use `#[cfg(test)]` which means `test_support` behind `feature = "test-support"` won't be available. Two options:
- Option A: Unit tests that need mocks import from `test_support` and rely on the dev-dependency self-reference enabling the feature during `cargo test`
- Option B: Keep mock definitions duplicated in the `#[cfg(test)]` block

Use Option A — the self-reference `loongclaw-kernel = { path = ".", features = ["test-support"] }` in dev-dependencies ensures the feature is active during `cargo test`.

**Step 7: Run tests**
```bash
cargo test -p loongclaw-kernel
```
Expected: All 36+ tests pass

**Step 8: Commit**
```bash
git add crates/kernel/
git commit -m "refactor(kernel): extract test_support module and migrate integration tests

- Add test-support feature with 6 mock adapters and helper functions
- Move integration tests (full kernel bootstrap) to crates/kernel/tests/
- Keep unit tests and proptest in src/tests.rs
- Update imports to use test_support re-exports"
```

---

## Phase 3: Application Layer (High Complexity)

### Task 6: app — extract test_support harness infrastructure

**Files:**
- Modify: `crates/app/src/test_support.rs` — expand with harness types (already exists with `ScopedEnv`)
- Modify: `crates/app/src/lib.rs` — change `#[cfg(test)] pub(crate) mod test_support` to `#[cfg(feature = "test-support")] pub mod test_support`
- Modify: `crates/app/Cargo.toml` — add test-support feature

**Step 1: Add `test-support` feature to Cargo.toml**

```toml
[features]
test-support = []

[dev-dependencies]
loongclaw-app = { path = ".", features = ["test-support"] }
```

**Step 2: Expand `src/test_support.rs`**

Add the harness types from `conversation/integration_tests.rs`:

```rust
//! Test support utilities for app crate.
//!
//! When `feature = "test-support"` is enabled, this module is `pub`.
//! When running `cargo test` on this crate, it's also available via `#[cfg(test)]`.

// Existing ScopedEnv stays...

// Re-export internal types needed by integration tests
pub use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
pub use crate::context::KernelContext;
pub use crate::tools::MvpToolAdapter;
pub use crate::tools::runtime_config::ToolRuntimeConfig;
pub use crate::tools::shell_policy_ext::ToolPolicyExtension;
pub use crate::tools::file_policy_ext::FilePolicyExtension;

#[cfg(feature = "memory-sqlite")]
pub use crate::memory::MvpMemoryAdapter;
#[cfg(feature = "memory-sqlite")]
pub use crate::memory::runtime_config::MemoryRuntimeConfig;

// Move FakeProviderBuilder here from integration_tests.rs
pub struct FakeProviderBuilder { /* ... exact copy ... */ }
impl FakeProviderBuilder { /* ... exact copy of all methods ... */ }

// Move TurnTestHarness here from integration_tests.rs
pub struct TurnTestHarness { /* ... exact copy ... */ }
impl TurnTestHarness { /* ... exact copy of new(), with_capabilities(), with_tool_config(), execute() ... */ }
impl Drop for TurnTestHarness { /* ... exact copy ... */ }
```

**Step 3: Update `lib.rs` visibility gate**

Change:
```rust
#[cfg(test)]
pub(crate) mod test_support;
```
To:
```rust
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
```

This ensures:
- `cargo test -p loongclaw-app` → test_support available (cfg(test))
- External `tests/` with dev-dep feature → test_support available (feature gate)
- Release builds → test_support excluded

**Step 4: Run tests (should still pass — no tests moved yet)**
```bash
cargo test -p loongclaw-app
```
Expected: All tests pass

**Step 5: Commit**
```bash
git add crates/app/src/test_support.rs crates/app/src/lib.rs crates/app/Cargo.toml
git commit -m "refactor(app): expand test_support with harness infrastructure

- Move FakeProviderBuilder and TurnTestHarness to test_support module
- Add test-support feature flag
- Re-export internal types needed by integration tests
- Gate with #[cfg(any(test, feature = 'test-support'))]"
```

---

### Task 7: app — migrate conversation integration tests to external `tests/`

**Files:**
- Create: `crates/app/tests/conversation_integration.rs` — 9+ integration tests
- Modify: `crates/app/src/conversation/mod.rs` — remove `mod integration_tests`
- Delete: `crates/app/src/conversation/integration_tests.rs` — fully relocated

**Step 1: Create `crates/app/tests/conversation_integration.rs`**

```rust
//! Integration tests for the conversation engine.
//!
//! These tests exercise real filesystem IO, real kernel bootstrap,
//! and real tool execution through TurnTestHarness.

use loongclaw_app::test_support::{FakeProviderBuilder, TurnTestHarness};
use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind};
use std::collections::BTreeSet;

// ... all 9+ integration test functions from integration_tests.rs
// Update: remove `use super::*` and `use super::integration_tests::*`
// Replace with: `use loongclaw_app::test_support::*`
```

**Step 2: Update `crates/app/src/conversation/mod.rs`**

Remove:
```rust
#[cfg(test)]
mod integration_tests;
```

Keep:
```rust
#[cfg(test)]
mod tests;
```

**Step 3: Delete `crates/app/src/conversation/integration_tests.rs`**

All content has been relocated to either `test_support.rs` (harness types) or `tests/conversation_integration.rs` (test functions).

**Step 4: Run tests**
```bash
cargo test -p loongclaw-app
```
Expected: All tests pass (integration tests now run from `tests/` dir)

**Step 5: Commit**
```bash
git add crates/app/tests/ crates/app/src/conversation/
git commit -m "refactor(app): migrate conversation integration tests to external tests/

- Move 9+ integration tests to crates/app/tests/conversation_integration.rs
- Remove integration_tests.rs from src/conversation/
- Tests now use loongclaw_app::test_support:: imports"
```

---

### Task 8: app — update conversation/tests.rs imports

**Files:**
- Modify: `crates/app/src/conversation/tests.rs` — update 8 import sites

**Step 1: Find and replace imports**

In `crates/app/src/conversation/tests.rs`, replace all occurrences of:
```rust
use super::integration_tests::TurnTestHarness;
```
With:
```rust
use crate::test_support::TurnTestHarness;
```

And similarly for `FakeProviderBuilder` if used:
```rust
use crate::test_support::FakeProviderBuilder;
```

**Step 2: Run tests**
```bash
cargo test -p loongclaw-app
```
Expected: All tests pass

**Step 3: Commit**
```bash
git add crates/app/src/conversation/tests.rs
git commit -m "refactor(app): update conversation/tests.rs imports to use test_support

- Replace 8 super::integration_tests:: imports with crate::test_support::"
```

---

### Task 9: app — audit and migrate remaining integration tests

**Files:**
- Read: `crates/app/src/provider/tests.rs` (2,872 lines)
- Read: `crates/app/src/memory/tests.rs` (277 lines)
- Potentially create: `crates/app/tests/provider_integration.rs`
- Potentially create: `crates/app/tests/memory_integration.rs`

**Step 1: Audit provider/tests.rs**

Classify each test:
- Uses real filesystem, real kernel, or real network → integration → move to `tests/`
- Uses only mocks and in-memory state → unit → keep inline

**Step 2: Audit memory/tests.rs**

Same classification. SQLite tests that create real `.db` files are integration tests.

**Step 3: Audit other modules**

Check `tools/`, `acp/`, `channel/`, `config/` for integration-style tests. The heuristic: temp dirs, real IO, real kernel bootstrap → move.

**Step 4: Move identified integration tests**

For each module with integration tests, create `crates/app/tests/<module>_integration.rs` and relocate.

**Step 5: Run full test suite**
```bash
cargo test -p loongclaw-app
```
Expected: All tests pass

**Step 6: Commit**
```bash
git add crates/app/
git commit -m "refactor(app): migrate remaining integration tests to external tests/

- Audit provider, memory, tools, acp, channel modules
- Move integration-style tests to crates/app/tests/
- Keep unit tests inline in src/"
```

---

## Phase 4: Daemon (Binary Crate — Special Handling)

### Task 10: daemon — split into lib + bin for external test access

**CRITICAL:** Daemon is a binary crate (`[[bin]]` in Cargo.toml). External `tests/` cannot import a binary crate's internals. We must split into `lib.rs` + `main.rs`.

**Files:**
- Create: `crates/daemon/src/lib.rs` — library root re-exporting all modules
- Modify: `crates/daemon/src/main.rs` — thin wrapper calling lib
- Modify: `crates/daemon/Cargo.toml` — add `[lib]` section + test-support feature

**Step 1: Update Cargo.toml**

Add library target alongside binary:
```toml
[lib]
name = "loongclaw_daemon"
path = "src/lib.rs"

[[bin]]
name = "loongclaw"
path = "src/main.rs"

[features]
test-support = []

[dev-dependencies]
loongclaw_daemon = { path = ".", features = ["test-support"] }
```

**Step 2: Create `src/lib.rs`**

Move all module declarations from `main.rs` to `lib.rs`. Export everything needed:
```rust
// All existing mod declarations from main.rs
pub mod mvp;
pub mod onboard_cli;
pub mod import_cli;
// ... etc

#[cfg(feature = "test-support")]
pub mod test_support;
```

**Step 3: Slim down `main.rs`**

```rust
use loongclaw_daemon::*;

fn main() {
    // ... only the entry point logic, everything else in lib.rs
}
```

**Step 4: Run tests to verify split**
```bash
cargo test -p loongclaw-daemon
cargo build -p loongclaw-daemon
```
Expected: Both pass

**Step 5: Commit**
```bash
git add crates/daemon/
git commit -m "refactor(daemon): split into lib + bin for external test access

- Create lib.rs with all module declarations
- Slim main.rs to thin entry point
- Add test-support feature flag
- Prerequisite for moving tests to external tests/ directory"
```

---

### Task 11: daemon — migrate `src/tests/` to `crates/daemon/tests/`

**Files:**
- Move: `crates/daemon/src/tests/` → `crates/daemon/tests/`
- Modify: `crates/daemon/src/lib.rs` — remove `#[cfg(test)] mod tests`
- Create: `crates/daemon/src/test_support.rs` — shared helpers from `tests/mod.rs`

**Step 1: Create `src/test_support.rs`**

Extract shared helpers from `src/tests/mod.rs`:
```rust
//! Test support utilities for daemon crate.

use std::sync::{Mutex, MutexGuard};

static DAEMON_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn lock_daemon_test_environment() -> MutexGuard<'static, ()> {
    DAEMON_TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub fn catalog_entry(raw: &str) -> crate::mvp::channel::ChannelCatalogEntry { /* ... */ }
pub fn channel_send_command(raw: &str) -> &'static str { /* ... */ }
pub fn approval_test_operation(tool_name: &str, payload: serde_json::Value) -> loongclaw_spec::OperationSpec { /* ... */ }
pub fn write_temp_risk_profile(path: &std::path::Path, body: &str) { /* ... */ }
pub fn sign_security_scan_profile_for_test(/* ... */) -> (String, String) { /* ... */ }
```

**Step 2: Move test files**

```bash
mkdir -p crates/daemon/tests
# Move each test file, updating imports
```

For each test file (e.g., `acp.rs`, `onboard_cli.rs`, etc.), update:
```rust
// Old: use super::*; use super::lock_daemon_test_environment;
// New:
use loongclaw_daemon::*;
use loongclaw_daemon::test_support::*;
```

**Step 3: Create new `tests/mod.rs` or individual test files**

Note: External `tests/` in Rust does NOT use `mod.rs`. Each `.rs` file is a separate test binary. For the 15 test files:

Option A: One file per test module (15 separate test binaries — slow compilation)
Option B: Single `tests/daemon_tests.rs` with `mod` declarations and a `tests/` subdirectory

Use Option B for compilation speed:
```
crates/daemon/tests/
├── daemon_tests.rs          # single entry point
├── daemon_tests/
│   ├── mod.rs               # declares submodules
│   ├── acp.rs
│   ├── architecture.rs
│   ├── doctor_feishu.rs
│   ├── feishu_cli.rs
│   ├── import_cli.rs
│   ├── migrate_cli.rs
│   ├── migration.rs
│   ├── onboard_cli.rs
│   ├── programmatic.rs
│   ├── skills_cli.rs
│   ├── spec_runtime.rs
│   └── spec_runtime_bridge/
│       ├── mod.rs
│       ├── process_stdio.rs
│       └── http_json.rs
```

Where `daemon_tests.rs`:
```rust
use loongclaw_daemon::test_support::*;
mod daemon_tests;
```

**Step 4: Remove old test module from lib.rs**

Remove:
```rust
#[cfg(test)]
mod tests;
```

**Step 5: Delete `src/tests/` directory**

**Step 6: Run tests**
```bash
cargo test -p loongclaw-daemon
```
Expected: All 100+ tests pass

**Step 7: Commit**
```bash
git add crates/daemon/
git commit -m "refactor(daemon): migrate src/tests/ to external crates/daemon/tests/

- Extract shared test helpers to src/test_support.rs
- Move 15 test files to crates/daemon/tests/daemon_tests/
- Single test binary entry point for compilation speed
- Update all imports from super:: to loongclaw_daemon::"
```

---

## Phase 5: Verification and Cleanup

### Task 12: workspace-wide verification

**Step 1: Full test suite**
```bash
cargo test --workspace
```
Expected: All tests pass

**Step 2: No test-support leak**
```bash
cargo test --workspace --no-default-features
```
Expected: All tests pass (test-support not active in non-test builds)

**Step 3: Check for orphaned pub(crate) test helpers**
```bash
grep -r 'pub(crate).*Test\|pub(crate).*Mock\|pub(crate).*Fake' crates/*/src/
```
Expected: No orphaned test helpers (all moved to test_support or removed)

**Step 4: Verify each tests/ file compiles independently**
```bash
for crate in protocol spec kernel app daemon; do
  cargo test -p loongclaw-$crate --tests 2>&1 | tail -1
done
```
Expected: All pass

**Step 5: Commit any remaining cleanup**
```bash
git add -A
git commit -m "chore: cleanup orphaned test helpers after convention migration"
```

---

### Task 13: Create PR

**Step 1: Push branch**
```bash
git push -u origin issue-139-test-convention-overhaul
```

**Step 2: Create PR**
```bash
gh pr create \
  --title "refactor: workspace-wide test convention overhaul" \
  --body "$(cat <<'EOF'
## Summary

Standardizes all 7 workspace crates on hybrid test convention per #139:

- **Inline** `#[cfg(test)] mod tests` for unit tests (stay in `src/`)
- **External** `crates/<name>/tests/` for integration tests
- **`feature = "test-support"`** gates internal type re-exports for external tests

### Per-Crate Changes

| Crate | Changes |
|-------|---------|
| contracts | No changes (0 tests) |
| bench | No changes (0 tests) |
| protocol | 21 tests → `tests/protocol_transport.rs` + `test_support` feature |
| spec | 2 tests → `tests/spec_execution.rs` + `test_support` feature |
| kernel | 36+ tests split: integration → `tests/`, unit stays inline, mock adapters → `test_support` |
| app | `TurnTestHarness` + `FakeProviderBuilder` → `test_support`, 9+ integration tests → `tests/`, 8 import sites updated |
| daemon | lib/bin split, `src/tests/` (15 files) → `crates/daemon/tests/`, shared helpers → `test_support` |

### Verification

- `cargo test --workspace` ✅
- `cargo test --workspace --no-default-features` ✅
- No orphaned `pub(crate)` test helpers
- Each `tests/` file compiles independently

Closes #139
EOF
)"
```

---

## Agent Parallelization Guide

Tasks that can run in parallel (independent crate work):

- **Parallel group 1:** Task 1 + Task 2 + Task 3 + Task 4 (leaf crates, no deps)
- **Parallel group 2:** Task 5 (kernel, depends on nothing)
- **Parallel group 3:** Task 6 + Task 7 + Task 8 (app, sequential within group)
- **Parallel group 4:** Task 9 (app audit, after Task 8)
- **Parallel group 5:** Task 10 + Task 11 (daemon, sequential within group)
- **Sequential:** Task 12 (verification, after all)
- **Sequential:** Task 13 (PR, after Task 12)

Recommended agent dispatch:
- Agent A: Tasks 1-4 (leaf crates)
- Agent B: Task 5 (kernel)
- Agent C: Tasks 6-9 (app)
- Agent D: Tasks 10-11 (daemon)
- Main: Task 12-13 (verification + PR)
