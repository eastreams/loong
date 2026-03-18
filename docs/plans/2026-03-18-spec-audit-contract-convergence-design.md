# Spec Audit Contract Convergence Design

## Problem

`#279` retired the unsafe implicit `NoopAuditSink` default from `LoongClawKernel::new()`, and the
app/runtime bootstrap path now has an explicit durable audit story. The remaining seam is `spec`:

1. `crates/spec/src/kernel_bootstrap.rs` still defaults its bootstrap builder to
   `InMemoryAuditSink` without naming that choice as a harness-specific contract.
2. `crates/spec/src/spec_execution.rs` constructs its own `InMemoryAuditSink` inline instead of
   reusing a shared spec-level audit decision.
3. `docs/SECURITY.md` already permits spec/demo helpers to stay in-memory, but the code path has no
   focused regression tests that prove this is an intentional harness default rather than an
   accidental drift.

The result is semantic ambiguity. Production-shaped app bootstraps are now explicit and durable by
default, while the `spec` layer remains silently in-memory.

## Goals

1. Make the `spec` audit default explicit as a harness-only in-memory policy.
2. Centralize `spec`'s in-memory audit construction so the contract lives in one place.
3. Add regression tests that fail if `spec` stops exposing in-memory audit events for report
   snapshots or if the bootstrap fallback stops being intentionally in-memory.
4. Align security/reliability docs with the resulting contract.

## Non-Goals

1. Do not route `spec` through app `[audit]` runtime configuration.
2. Do not introduce a new cross-layer audit profile abstraction.
3. Do not change production CLI/Telegram/Feishu audit defaults in this slice.

## Options Considered

### A. Promote `spec` to production-shaped durable audit defaults

This would thread app-style audit configuration into `spec` and make `spec` mirror runtime
bootstraps.

Why not now:
- `spec` is intentionally detached from `app`.
- Durable retention is an operator/runtime concern, while `spec` is still used as a harness/demo
  path and for side-effect-free evaluation.
- The change set would be much larger and would blur an architecture boundary just to remove a
  smaller semantic ambiguity.

### B. Keep `spec` in-memory but leave the current implicit defaults in place

This is the smallest code delta, but it preserves the root problem: callers still have to infer
intent from scattered `InMemoryAuditSink::default()` allocations and builder fallbacks.

Why not:
- It does not create a durable regression guard.
- It keeps docs and code coupled only by tribal knowledge.

### C. Keep `spec` in-memory and make that contract explicit

Recommended.

Concretely:
- add a named helper in `crates/spec/src/kernel_bootstrap.rs` for the spec/harness in-memory audit
  default
- use that helper everywhere `spec` needs its default sink
- document that `KernelBuilder`/`BootstrapBuilder` are harness bootstrap helpers whose implicit
  fallback is intentionally in-memory
- add tests that exercise real `spec` execution with `include_audit = true`

Why this is the right cut:
- minimal change set
- no new hardcoding or policy duplication
- keeps `spec` detached from `app`
- mechanically protects the intended semantics

## Proposed Design

### 1. Introduce a named spec audit helper

Add a small helper in `crates/spec/src/kernel_bootstrap.rs`:

- `pub fn default_in_memory_audit_sink() -> Arc<InMemoryAuditSink>`

This gives the harness default a single source of truth. The helper name carries the architectural
meaning that raw `InMemoryAuditSink::default()` does not.

### 2. Reuse the helper in both bootstrap and spec execution

Replace direct inline `InMemoryAuditSink::default()` construction in:

- `crates/spec/src/kernel_bootstrap.rs`
- `crates/spec/src/spec_execution.rs`

with the named helper.

That keeps the fallback policy centralized and avoids semantic drift between builder bootstraps and
the report-producing spec execution path.

### 3. Add regression coverage at the behavior seam

Add tests in:

- `crates/spec/src/kernel_bootstrap.rs`
- `crates/spec/tests/spec_execution.rs`

Coverage:
- the default spec audit helper records real kernel events
- `execute_spec(..., include_audit = true)` returns captured audit events instead of `None`
- `execute_spec(..., include_audit = false)` continues suppressing snapshots

This gives us a behavior-level guard instead of only a comment-level contract.

### 4. Tighten docs

Update:

- `docs/SECURITY.md`
- `docs/RELIABILITY.md`

to say that:
- production-shaped app bootstraps default to durable audit modes
- `spec`/demo/harness helpers intentionally default to in-memory audit for side-effect-free local
  execution and snapshot reporting

## Testing Strategy

1. Write failing spec tests first for audit snapshot inclusion/suppression.
2. Add a focused bootstrap test proving the named helper captures emitted audit events.
3. Run targeted spec/kernel tests during iteration.
4. Run full repo verification before declaring completion.

## Risks

1. Over-correcting by threading app audit configuration into `spec` would widen the slice and create
   boundary drift.
2. Only updating docs without behavior tests would recreate the same ambiguity later.

## Recommendation

Implement Option C. It is the smallest change that closes the actual gap: `spec` stays
harness-oriented and in-memory by design, but that design becomes explicit, centralized, tested, and
documented.
