# Retire the Implicit Noop Audit Default Design

Date: 2026-03-18
Branch: `feat/279-retire-noop-audit-default`
Scope: constructor-level kernel audit safety on `alpha-test`
Linked issue: `#279`
Status: proposed direction, pre-implementation design

## Problem

`alpha-test` now treats audit as a first-class kernel concern in the main runtime/bootstrap path, but
the most convenient kernel constructor still undermines that story:

1. `LoongClawKernel::new()` still wires `NoopAuditSink`.
2. the local repository only calls `LoongClawKernel::new()` from tests today, which hid the risk
   while the production bootstrap moved forward
3. external or future internal callers can still reach for `new()` and silently drop security-
   critical audit events unless they know to switch to `with_runtime(...)`

This is a constructor-boundary truth gap. The repository docs already say silent audit drops are
bugs, but the public default constructor still encodes the opposite behavior.

## Root Cause

`git log -S'NoopAuditSink' -- crates/kernel/src/kernel.rs` traces the default back to commit
`7574d362` (`feat: bootstrap layered kernel, security governance, and roadmap`). At that point the
kernel needed a side-effect-free constructor quickly, so `new()` took the shortest path and reused
`NoopAuditSink`.

That shortcut outlived the original context:

1. audit retention now exists as a real architectural lane
2. repository documentation has been updated to describe audit as non-optional for governed paths
3. the constructor default never got revisited, so the safety model and the API default drifted

The root problem is not missing documentation or missing durable retention anymore. The root
problem is that the public default constructor still encodes a historical bootstrap shortcut.

## Goals

1. Make the default kernel constructor stop silently dropping audit events.
2. Preserve additive API compatibility for callers that already use `LoongClawKernel::new()`.
3. Keep an explicit no-audit path for narrow fixtures that intentionally do not need audit.
4. Make repository-owned test callsites grep-able about their audit intent.
5. Keep the change tightly scoped to constructor semantics and audit docs.

## Non-Goals

1. Do not redesign app/bootstrap durable audit retention.
2. Do not add new storage backends, query APIs, or audit indexing.
3. Do not change kernel event schemas or policy semantics.
4. Do not break the existing public constructor signature.

## Current State

### What is already correct

1. `AuditSink` is a small injectable seam.
2. `InMemoryAuditSink` already exists and is adequate for tests and local assertions.
3. production-shaped app bootstrap already assembles explicit audit sinks outside the kernel.

### What is still wrong

1. `LoongClawKernel::new()` silently selects `NoopAuditSink`.
2. `KernelBuilder::new()` inherits the same default because it aliases `LoongClawKernel`.
3. local tests mostly use `new()`, which keeps audit intent implicit inside the repository.

## Approaches Considered

### A. Keep `new()` as-is and only document the exception

Pros:

1. no code churn
2. zero compatibility risk

Cons:

1. preserves the root footgun
2. keeps the public API in conflict with the repository's security claims
3. relies on contributor memory instead of mechanical defaults

### B. Change `new()` to use `InMemoryAuditSink`, add an explicit no-audit constructor

Pros:

1. additive API change
2. smallest behavioral fix that makes the default safe
3. keeps side-effect-free fixtures available through an intentionally named escape hatch
4. aligns the default constructor with current docs and runtime expectations

Cons:

1. changes constructor behavior for callers who implicitly relied on audit being discarded
2. does not force every caller to choose a sink explicitly at compile time

### C. Remove or change the `new()` signature so audit is always caller-provided

Pros:

1. strongest explicitness
2. impossible to forget audit at the constructor boundary

Cons:

1. breaking API change against the repository's additive-compatibility rule
2. larger migration surface than the actual risk warrants
3. not necessary because the repo currently has only test callsites

## Decision

Implement Approach B.

Specifically:

1. change `LoongClawKernel::new()` to use `InMemoryAuditSink`
2. add a deliberately named explicit no-audit constructor for narrow fixture use
3. add an explicit in-memory helper that returns the sink handle for audit assertions
4. migrate repository test callsites away from ambiguous `new()` where direct audit intent can be
   made explicit with minimal churn

This is the smallest fix that closes the constructor-level safety gap without breaking public API
shape.

## Why This Is Better Than a Heavier Refactor

The tempting stricter design is to require every caller to pass an `Arc<dyn AuditSink>`. That would
make audit choice explicit everywhere, but it is the wrong move for this slice:

1. it violates the repository's additive-API rule
2. it creates migration noise unrelated to the actual bug
3. the real defect is the unsafe default, not the existence of a convenience constructor

Using `InMemoryAuditSink` as the safe default keeps the default constructor honest while preserving
the explicit `with_runtime(...)` seam for full control.

## Validation Strategy

1. Add regression coverage proving the default constructor lane is backed by in-memory audit.
2. Add regression coverage for the explicit no-audit constructor.
3. Update internal tests to use explicit constructor helpers where that improves audit intent.
4. Reconcile reliability/core-belief docs with the actual constructor behavior.

## Expected Outcome

After this slice:

1. the default kernel constructor no longer silently drops audit events
2. `NoopAuditSink` remains available, but only behind an explicit constructor name
3. repository-owned tests become clearer about whether they want in-memory audit or no audit
4. the kernel audit story becomes internally consistent again
