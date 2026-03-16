# Persistent Kernel Audit Sink Design

Date: 2026-03-15
Branch: `arch/alpha-test-kernel-runtime-next`
Scope: kernel-first durable audit retention for `alpha-test`
Linked issue: `#172`
Status: proposed direction, pre-implementation design

## Problem

`alpha-test` now emits typed audit events for the right kinds of security-critical actions, but it
still treats audit retention as process-local state. The core seam is already present in
`crates/kernel/src/audit.rs`: `AuditSink` is a small trait, `InMemoryAuditSink` provides test-time
snapshots, and kernel operations propagate `AuditError` instead of silently swallowing sink
failures. The missing piece is durability.

Today the repository makes three claims that are only partially true in code:

1. `ARCHITECTURE.md` says security-critical decisions should be auditable.
2. `docs/design-docs/layered-kernel-design.md` places audit in a kernel-owned observability layer.
3. `docs/RELIABILITY.md` says bootstrap paths use `InMemoryAuditSink` or better.

Those claims are directionally correct, but they still leave `alpha-test` with a material operator
gap:

1. policy denials disappear on process restart
2. token issuance and revocation history disappears on process restart
3. security-scan summary events disappear on process restart
4. human approval and policy enforcement can happen correctly in real time, but the historical
   evidence trail is not durable

This is exactly the wrong place to remain soft. If LoongClaw is going to act like a kernel, audit
retention cannot stay a test-only convenience.

## Goals

1. Make security-critical audit evidence durable across process restarts.
2. Preserve the kernel as the source of truth for audit event emission.
3. Keep the stable audit contract small and additive.
4. Avoid silently downgrading from durable audit to in-memory audit in production paths.
5. Keep test and local diagnostics workflows simple by preserving an in-memory snapshot option.
6. Keep the first slice reviewable and independent of PR `#169`.

## Non-Goals

1. Do not redesign the authorization model tracked in `#48`.
2. Do not introduce a full analytics/query subsystem in the kernel.
3. Do not add a relational storage contract or migration framework in the first slice.
4. Do not retune approval policy, risk scoring, or token semantics here.
5. Do not fold checkpoint governance or conversation coordinator extraction into this issue.

## Current State

### What already exists

1. `crates/kernel/src/audit.rs` defines a minimal `AuditSink` trait and an in-memory sink.
2. `crates/kernel/src/kernel.rs` records audit events for token issuance, token revocation,
   task dispatch, and other governed execution paths.
3. `crates/spec/src/kernel_bootstrap.rs` already treats audit as an injected bootstrap concern,
   which means the architecture has a seam for alternate sink implementations.
4. `crates/app/src/config/shared.rs` already defines `~/.loongclaw` as the default runtime home,
   so the repository has an established operator-owned state root for local persistence.

### What is still wrong

1. The default bootstrapped runtime still uses `InMemoryAuditSink`, so the production-shaped path
   has no durable audit evidence.
2. There is no kernel-owned append-only journal implementation.
3. There is no fanout/composite sink that keeps test-friendly snapshot behavior while adding
   durability.
4. There is no explicit audit runtime configuration surface for choosing durable behavior at
   bootstrap time.
5. There is no documented operator workflow for inspecting persisted audit evidence outside the
   in-memory demo path.

## Why This Slice Comes Before Other Refactors

The strongest alternative next step is to continue extracting control-plane complexity out of
`ConversationTurnCoordinator`, especially checkpoint governance and approval orchestration. That
work is real and should happen, but it is not the best next slice for this branch.

The durable-audit track wins first for three reasons:

1. it strengthens the kernel's most security-sensitive evidence boundary
2. it does not depend on the unmerged runtime-binding stack in PR `#169`
3. it improves both the current token/policy model and any later handle-based model from `#48`

Said differently: coordinator extraction improves control-plane shape; durable audit improves the
truthfulness of the kernel itself.

## External Calibration

As a calibration point, the open-source `openai/codex` runtime keeps execution authority, approval
decisions, and sandboxing under an explicit orchestration/runtime boundary rather than treating
them as product-only concerns. The useful lesson for LoongClaw is not to copy Codex's exact
components, but to preserve the same architectural priority:

1. approval and execution policy belong to the runtime core
2. runtime evidence must survive long enough to be inspected
3. product-facing workflows sit above those primitives instead of replacing them

That supports a kernel-first durable audit lane rather than an operator-only reporting feature.

## Approaches Considered

### A. SQLite-first persistent audit

Add a SQLite-backed audit table as the primary durable sink in the first slice.

Pros:

1. immediately queryable
2. easy to filter by event kind, actor, or time range
3. could reuse the repository's existing `rusqlite` footprint

Cons:

1. couples the first persistence slice to schema design and migration rules
2. risks turning audit retention into a storage-feature discussion instead of a kernel-boundary
   improvement
3. raises the cost of later changing the audit-read surface

### B. Append-only JSONL plus fanout

Add a deterministic append-only JSONL sink and a small fanout sink that can write to both JSONL
and in-memory snapshot storage.

Pros:

1. smallest additive change to the kernel-owned audit seam
2. no schema migration burden in slice 1
3. naturally preserves event ordering
4. easy operator inspection with standard tools
5. keeps the kernel focused on event durability, not analytics

Cons:

1. query ergonomics are weaker than SQLite
2. future compaction or rotation needs a follow-on policy

### C. Remote or SIEM-only delivery

Treat durable audit as an export concern and skip local persistence.

Pros:

1. aligns with future enterprise operations
2. avoids local file management

Cons:

1. fails when transport is unavailable or intentionally disabled
2. weakens local postmortem and debugging on `alpha-test`
3. adds operational complexity before local durability is solved

## Decision

Implement Approach B first.

The right first move is a kernel-first append-only journal with optional in-memory fanout. That
gives `alpha-test` a durable evidence lane now without freezing a relational read model too early.

## Target Design

### 1. Keep `AuditSink` small and additive

The existing trait shape is already close to correct:

```rust
pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError>;
}
```

The first slice should preserve that surface. The kernel should continue to emit typed audit
events without learning about file paths, query APIs, or operator UI concerns.

### 2. Add two new sink implementations

The kernel crate should gain two additive sink implementations:

1. `JsonlAuditSink`
2. `FanoutAuditSink`

Recommended behavior:

1. `JsonlAuditSink` appends one canonical JSON line per `AuditEvent`
2. writes are serialized under a small internal mutex to preserve per-process ordering
3. the sink creates the parent directory when bootstrapped, but initialization fails if the path
   cannot be prepared
4. write failures return `AuditError::Sink(...)`, which preserves the current fail-closed contract
5. `FanoutAuditSink` forwards the same event to a fixed set of child sinks in order

`InMemoryAuditSink` should stay in place for tests, demos, and snapshot assertions.

### 3. Split storage concerns cleanly between kernel and bootstrap code

The kernel should own event emission and sink semantics. App and daemon code should own runtime
path resolution and configuration.

That means:

1. sink implementations can live in `crates/kernel/src/audit.rs`
2. the chosen audit file path should be resolved in config/bootstrap layers
3. production bootstrap helpers should assemble the sink graph explicitly
4. test-only builders can keep `InMemoryAuditSink` defaults where side-effect-free behavior matters

This keeps the kernel authoritative without turning it into a config loader.

### 4. Add an explicit audit runtime config surface

The first slice should add a top-level audit config section to `LoongClawConfig`:

```toml
[audit]
mode = "fanout"        # in_memory | jsonl | fanout
path = "~/.loongclaw/audit/events.jsonl"
retain_in_memory = true
```

Recommended semantics:

1. `in_memory` stays test/demo friendly
2. `jsonl` writes only to the durable journal
3. `fanout` writes to both the journal and an in-memory sink
4. `retain_in_memory` is meaningful only for `fanout`

The config surface should be small. Rotation, retention windows, and external export configuration
can stay out of slice 1.

### 5. Make production-shaped bootstrap durable by default

The first slice should not silently keep the existing in-memory-only runtime in main entrypoints.
The intended production-shaped bootstraps should default to `fanout` under the LoongClaw home
directory.

Recommended defaults:

1. path: `~/.loongclaw/audit/events.jsonl`
2. mode: `fanout`
3. tests/spec builders: explicit `in_memory` unless a test opts into durability

This keeps local operator behavior simple while avoiding a silent downgrade in the main runtime.

### 6. Preserve fail-closed semantics

Audit durability is only meaningful if failures are visible.

The current kernel behavior already propagates sink errors through operation results. The new
design should preserve that property:

1. sink initialization errors fail bootstrap
2. append failures fail the governed operation that tried to emit the event
3. no automatic fallback from `jsonl` or `fanout` to `in_memory`

The point of this slice is not "best effort telemetry." It is "durable evidence or explicit
failure."

### 7. Keep operator inspection simple in slice 1

The first slice does not need a dedicated query CLI. The design should document a basic
operator-facing workflow:

1. journal lives under `~/.loongclaw/audit/events.jsonl`
2. each line is one canonical `AuditEvent`
3. local inspection can use `tail`, `jq`, or a small future wrapper command

If a later slice adds `loongclaw audit print` or a SQLite read model, it can build on the same
durable journal instead of replacing it.

## Testing Strategy

1. Kernel unit tests:
   - JSONL sink persists ordered events to disk
   - fanout sink writes to both children
   - JSONL sink surfaces write errors
2. Bootstrap/config tests:
   - audit config resolves the default path under `~/.loongclaw`
   - daemon/app bootstrap chooses the configured sink mode
3. End-to-end runtime tests:
   - a security-critical event survives process boundaries
   - event shape stays compatible with existing typed schema

## Acceptance Criteria

1. `alpha-test` has a durable local audit journal implementation.
2. Production-shaped bootstrap paths default to durable audit retention.
3. No governed operation silently downgrades to in-memory-only audit on durable sink failure.
4. Existing audit event schema remains additive and stable.
5. Tests still have a clean in-memory snapshot path for assertions and demos.
6. Docs explain both the kernel boundary and the operator inspection workflow.

## Follow-On Work

After this lands, the next adjacent architecture slices should be:

1. audit-read ergonomics (`loongclaw audit print` or equivalent)
2. optional SQLite projection or filtered snapshot adapter
3. remote export / SIEM adapters
4. conversation control-plane extraction from `ConversationTurnCoordinator`
5. broader authorization redesign from `#48`
