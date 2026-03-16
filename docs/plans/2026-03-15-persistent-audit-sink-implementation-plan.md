# Persistent Kernel Audit Sink Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a durable kernel-owned audit journal for `alpha-test`, with production-shaped bootstraps defaulting to durable retention while tests and demos keep an explicit in-memory path.

**Architecture:** Keep `AuditSink` as the stable kernel seam. Add additive sink implementations for append-only JSONL persistence and fanout, then let config/bootstrap code choose `in_memory`, `jsonl`, or `fanout` without changing kernel event emission semantics. Preserve fail-closed behavior: if durable audit is selected and the sink cannot write, the governed operation must fail rather than silently downgrading.

**Tech Stack:** Rust, serde JSON serialization, filesystem I/O, LoongClaw config/bootstrap code, `cargo test`, `cargo clippy`

---

### Task 1: Lock the design docs and evidence trail

**Files:**
- Create: `docs/plans/2026-03-15-persistent-audit-sink-design.md`
- Create: `docs/plans/2026-03-15-persistent-audit-sink-implementation-plan.md`

**Step 1: Re-read the current audit seam**

Run: `rg -n "AuditSink|InMemoryAuditSink|record_audit_event|issue_token|revoke_token" crates/kernel crates/spec crates/app`
Expected: the current kernel emission path and in-memory-only retention points are enumerated.

**Step 2: Confirm the plan files exist**

Run: `ls docs/plans/2026-03-15-persistent-audit-sink-design.md docs/plans/2026-03-15-persistent-audit-sink-implementation-plan.md`
Expected: both files exist.

### Task 2: Write the failing kernel tests for durability

**Files:**
- Modify: `crates/kernel/src/tests.rs`
- Test: `crates/kernel/src/tests.rs`

**Step 1: Add a failing JSONL persistence test**

Add a test that records two audit events through a `JsonlAuditSink` and asserts that:
- the journal file exists
- it contains two lines
- the event order matches emission order

**Step 2: Add a failing fanout test**

Add a test that records one audit event through a `FanoutAuditSink` composed of:
- one `InMemoryAuditSink`
- one `JsonlAuditSink`

Assert that both child sinks observe the event.

**Step 3: Add a failing write-error propagation test**

Add a test that points the JSONL sink at an unwritable path or directory and asserts the sink
returns an `AuditError::Sink(...)` instead of silently succeeding.

**Step 4: Run the targeted tests and confirm RED**

Run: `cargo test -p loongclaw-kernel jsonl_audit_sink_ -- --test-threads=1`
Expected: FAIL because the new sink types do not exist yet.

### Task 3: Implement additive durable sink types in the kernel crate

**Files:**
- Modify: `crates/kernel/src/audit.rs`
- Modify: `crates/kernel/src/lib.rs`
- Modify: `crates/kernel/src/tests.rs`

**Step 1: Add `JsonlAuditSink`**

Implement a sink that:
- opens or creates the journal file
- appends one canonical JSON line per event
- serializes writes behind a mutex so concurrent callers preserve line integrity

**Step 2: Add `FanoutAuditSink`**

Implement a sink that forwards one event to a fixed ordered list of child sinks and fails on the
first child error.

**Step 3: Export the new sink types**

Re-export the new sink types from `crates/kernel/src/lib.rs` so bootstrap code can construct them
without reaching into internal modules.

**Step 4: Run the kernel-focused tests**

Run: `cargo test -p loongclaw-kernel jsonl_audit_sink_ fanout_audit_sink_ -- --test-threads=1`
Expected: PASS.

### Task 4: Add audit runtime configuration and bootstrap wiring

**Files:**
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/context.rs`
- Modify: `crates/spec/src/kernel_bootstrap.rs`
- Modify: `crates/daemon/src/main.rs`

**Step 1: Add the audit config model**

Introduce the smallest config surface needed for:
- `mode = in_memory | jsonl | fanout`
- `path`
- optional `retain_in_memory`

Keep defaults aligned with the design doc.

**Step 2: Add a bootstrap-level sink factory**

Create one helper that:
- resolves the audit path under `~/.loongclaw` when unset
- constructs the configured sink graph
- returns a typed sink object or a bootstrap error

**Step 3: Update production-shaped bootstrap paths**

Wire the helper into:
- app-level kernel bootstrap in `crates/app/src/context.rs`
- runtime builders used by daemon entrypoints

Keep explicit in-memory overrides for tests and demos where side-effect-free behavior is required.

**Step 4: Run focused bootstrap/config tests**

Run: `cargo test -p loongclaw-app audit_ -- --test-threads=1`
Expected: PASS once the config and factory path is covered.

### Task 5: Update docs for the new audit contract

**Files:**
- Modify: `docs/SECURITY.md`
- Modify: `docs/RELIABILITY.md`
- Modify: `README.md` if operator-facing default behavior needs a short note

**Step 1: Reconcile the security docs**

Update the docs so they say:
- typed audit events already exist
- durable retention is now available
- production-shaped bootstraps default to the configured durable mode

**Step 2: Document the operator inspection workflow**

Describe:
- default journal path
- one-event-per-line JSON format
- simple local inspection commands

**Step 3: Check the scoped doc diff**

Run: `git diff -- docs/SECURITY.md docs/RELIABILITY.md README.md`
Expected: only the intended audit-retention updates are present.

### Task 6: Run full verification and prepare delivery

**Files:**
- Modify: `crates/kernel/src/audit.rs`
- Modify: `crates/kernel/src/lib.rs`
- Modify: `crates/kernel/src/tests.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/context.rs`
- Modify: `crates/spec/src/kernel_bootstrap.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `docs/SECURITY.md`
- Modify: `docs/RELIABILITY.md`
- Modify: `README.md` if needed
- Create: `docs/plans/2026-03-15-persistent-audit-sink-design.md`
- Create: `docs/plans/2026-03-15-persistent-audit-sink-implementation-plan.md`

**Step 1: Run kernel tests**

Run: `cargo test -p loongclaw-kernel -- --test-threads=1`
Expected: PASS.

**Step 2: Run app tests**

Run: `cargo test -p loongclaw-app -- --test-threads=1`
Expected: PASS.

**Step 3: Run full-feature workspace tests**

Run: `cargo test --workspace --all-features -- --test-threads=1`
Expected: PASS.

**Step 4: Run lint**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS.

**Step 5: Review the final scoped diff**

Run: `git diff -- crates/kernel/src/audit.rs crates/kernel/src/lib.rs crates/kernel/src/tests.rs crates/app/src/config/runtime.rs crates/app/src/config/mod.rs crates/app/src/context.rs crates/spec/src/kernel_bootstrap.rs crates/daemon/src/main.rs docs/SECURITY.md docs/RELIABILITY.md README.md docs/plans/2026-03-15-persistent-audit-sink-design.md docs/plans/2026-03-15-persistent-audit-sink-implementation-plan.md`
Expected: only the intended persistent-audit slice is present.
