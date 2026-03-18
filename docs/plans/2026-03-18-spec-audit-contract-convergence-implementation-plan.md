# Spec Audit Contract Convergence Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the `spec` layer's in-memory audit default explicit, centralized, and regression-tested without widening the `spec -> app` boundary.

**Architecture:** Treat `spec` as a harness/demo bootstrap surface with an intentionally in-memory audit default. Centralize that choice behind a named helper, reuse it in builder and execution paths, and verify the behavior through end-to-end spec execution tests plus focused bootstrap tests.

**Tech Stack:** Rust workspace, `cargo test`, spec/kernel crates, Markdown design/reliability/security docs.

---

### Task 1: Add the failing spec execution audit tests

**Files:**
- Modify: `crates/spec/tests/spec_execution.rs`

**Step 1: Write the failing test**

Add:
- one async test asserting `execute_spec(&spec, true)` returns `Some(audit_events)` with at least one event
- one async test asserting `execute_spec(&spec, false)` returns `None`

Use a minimal `ToolCore` runner spec that succeeds through the normal spec path.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-spec --test spec_execution execute_spec_returns_audit_snapshot_when_requested -- --exact`

Expected: FAIL because the current behavior is not yet expressed through focused regression coverage.

**Step 3: Write minimal implementation**

Do not change production code yet. Only fix compile issues in the new tests if needed.

**Step 4: Run test to verify it still fails for the intended reason**

Run: `cargo test -p loongclaw-spec --test spec_execution execute_spec_returns_audit_snapshot_when_requested -- --exact`

Expected: FAIL on the assertion, not due to unrelated compile errors.

**Step 5: Commit**

Do not commit yet. This slice will be committed after code + docs are green.

### Task 2: Centralize the spec in-memory audit default

**Files:**
- Modify: `crates/spec/src/kernel_bootstrap.rs`
- Modify: `crates/spec/src/spec_execution.rs`

**Step 1: Add the named helper**

Add a helper such as:

```rust
pub fn default_in_memory_audit_sink() -> Arc<InMemoryAuditSink> {
    Arc::new(InMemoryAuditSink::default())
}
```

**Step 2: Route bootstrap fallback through the helper**

Replace inline `Arc::new(InMemoryAuditSink::default())` fallback construction in
`configured_builder(...)` with the helper.

**Step 3: Route spec execution through the helper**

Replace the inline `audit_sink` construction in `execute_spec_with_native_tool_executor(...)` with
the helper so spec execution and spec bootstrap share one default.

**Step 4: Run the focused spec test**

Run: `cargo test -p loongclaw-spec --test spec_execution execute_spec_returns_audit_snapshot_when_requested -- --exact`

Expected: PASS

**Step 5: Run the suppression test**

Run: `cargo test -p loongclaw-spec --test spec_execution execute_spec_suppresses_audit_snapshot_when_not_requested -- --exact`

Expected: PASS

### Task 3: Add a focused bootstrap regression test

**Files:**
- Modify: `crates/spec/src/kernel_bootstrap.rs`

**Step 1: Write the failing test**

Add a unit test that:
- constructs the named spec audit helper
- wires it into `KernelBuilder`
- issues a token
- asserts the in-memory audit snapshot is non-empty

**Step 2: Run test to verify it fails or is missing**

Run: `cargo test -p loongclaw-spec builder_explicit_in_memory_audit_records_events -- --exact`

Expected: FAIL before the helper/test wiring is complete.

**Step 3: Write minimal implementation**

Use the new helper in the test and keep the production code limited to the centralization from Task 2.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-spec builder_explicit_in_memory_audit_records_events -- --exact`

Expected: PASS

### Task 4: Update the documentation contract

**Files:**
- Modify: `docs/SECURITY.md`
- Modify: `docs/RELIABILITY.md`
- Create: `docs/plans/2026-03-18-spec-audit-contract-convergence-design.md`
- Create: `docs/plans/2026-03-18-spec-audit-contract-convergence-implementation-plan.md`

**Step 1: Update security wording**

Clarify that durable retention exists on production-shaped app bootstraps, while `spec`/demo
helpers intentionally remain in-memory for side-effect-free execution and audit snapshot reporting.

**Step 2: Update reliability wording**

Clarify that the "never silently dropped" invariant means:
- production paths default to durable or in-memory-backed audit as documented
- explicit no-audit paths must remain opt-in only
- `spec` defaults are intentionally in-memory and named as such

**Step 3: Review for consistency**

Run: `rg -n "No persistent audit sink|In-memory only|spec.*in-memory|fanout|new_without_audit" docs crates/spec`

Expected: wording reflects the current contract with no stale contradiction.

### Task 5: Run verification and prepare the final change set

**Files:**
- Modify: `crates/spec/src/kernel_bootstrap.rs`
- Modify: `crates/spec/src/spec_execution.rs`
- Modify: `crates/spec/tests/spec_execution.rs`
- Modify: `docs/SECURITY.md`
- Modify: `docs/RELIABILITY.md`
- Create: `docs/plans/2026-03-18-spec-audit-contract-convergence-design.md`
- Create: `docs/plans/2026-03-18-spec-audit-contract-convergence-implementation-plan.md`

**Step 1: Run focused tests**

Run:
- `cargo test -p loongclaw-spec --test spec_execution`
- `cargo test -p loongclaw-spec kernel_bootstrap --lib`

Expected: PASS

**Step 2: Run repo verification**

Run:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace -- --test-threads=1`
- `cargo test --workspace --all-features -- --test-threads=1`
- `./scripts/check_architecture_boundaries.sh`
- `./scripts/check_dep_graph.sh`
- `./scripts/check-docs.sh`

Expected: PASS, except for known unrelated non-blocking release-artifact warnings if they remain unchanged.

**Step 3: Review the final diff for scope**

Run:
- `git diff -- crates/spec/src/kernel_bootstrap.rs crates/spec/src/spec_execution.rs crates/spec/tests/spec_execution.rs docs/SECURITY.md docs/RELIABILITY.md docs/plans/2026-03-18-spec-audit-contract-convergence-design.md docs/plans/2026-03-18-spec-audit-contract-convergence-implementation-plan.md`

Expected: only the intended audit-contract convergence slice is present.
