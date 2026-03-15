# Issue 128 Approval Attention Rebuild Design

## Goal

Rebuild the approval request lifecycle and operator attention surface for issue `#128` directly on
top of current `alpha-test`, then expose a mergeable, auditable tool surface that fits the modern
session/runtime architecture.

## Problem

The closed PR `#129` tried to extend an approval system that no longer matches current
`alpha-test`. Its branch is structurally stale against today's session/runtime contracts and cannot
be repaired with a mechanical rebase.

At the same time, issue `#128` is still valid at the product level: operators need a canonical
approval queue, request status inspection, decision resolution, and a single attention view that
combines execution-side and grant-side signals.

Current `alpha-test` already provides useful foundations:

- session persistence and lineage in `crates/app/src/session/repository.rs`
- app-tool routing via `DefaultAppToolDispatcher`
- session visibility rules used by existing session tools
- conversation persistence hooks and durable `session_events`

What it still does not provide is the governed approval lifecycle itself:

- no durable approval request record
- no durable runtime grant record
- no approval tool surface
- no operator attention union view
- no automatic resume path after operator approval

## Constraints

- `alpha-test` is the source of truth.
- The rebuild must align with current app-tool/session architecture instead of reviving removed
  request-approval codepaths.
- The implementation must stay auditable and avoid hardcoded one-off behavior.
- TDD is required: failing tests first, then minimal implementation.
- Validation must include targeted tests, formatting, clippy, and full workspace tests before any
  GitHub delivery step.

## Approaches Considered

### Approach A: Event-only attention reconstruction

Represent approval state purely by replaying `session_events`, then expose a thin query view that
derives pending requests and attention heuristics from event history.

Pros:

- smallest immediate patch
- maximally reuses existing event storage

Cons:

- request identity and lifecycle remain implicit
- idempotent operator resolution becomes fragile
- grant persistence is awkward
- attention semantics become heuristic rather than authoritative

### Approach B: Minimal durable lifecycle plus derived attention

Add explicit durable approval request and runtime grant records, wire `TurnEngine` and app-tool
runtime to materialize and resolve those records, then derive canonical attention summaries from
durable state plus lifecycle events.

Pros:

- explicit request state machine
- stable request IDs and replay-safe resolution
- real `approve_once` / `approve_always` / `deny` behavior
- attention can be derived from authoritative lifecycle state
- fits current `SessionRepository` and session-tool visibility model

Cons:

- moderate scope
- requires runtime support for post-approval execution

### Approach C: Full port of the old attention-heavy PR surface

Port the old large `approval.rs` implementation and its full filter/summarization matrix into
current `alpha-test`.

Pros:

- richest surface immediately

Cons:

- highest drift risk
- old implementation assumes removed contracts and stale modules
- several integrity signals in the old code rely on bounded event windows and weaker invariants

## Recommendation

Use Approach B.

This is the smallest slice that restores a truthful approval runtime while still solving the
operator problem from issue `#128`. It preserves the "few thick primitives" design direction:

- one durable request object
- one durable runtime grant object
- one narrow approval tool surface
- one canonical attention view derived from execution and grant signals

The old PR remains reference material only, not a port target.

## Proposed Architecture

### 1. Durable approval request and grant state in `SessionRepository`

Extend the SQLite-backed control-plane store with two approval-specific tables:

- `approval_requests`
- `approval_grants`

`approval_requests` stores the lifecycle of a blocked governed tool call, including:

- deterministic `approval_request_id`
- session, turn, and tool-call correlation
- canonical tool name and approval key
- request payload snapshot for replay
- governance snapshot for auditability
- explicit status and decision fields
- timestamps for request, resolution, and execution
- last error for failed resumed execution

`approval_grants` stores session-lineage-scoped runtime grants for `approve_always`.

The repository remains the single durable backend. No separate approval repository layer is needed
for this slice.

### 2. Governed approval materialization inside `TurnEngine`

When a governed tool call requires approval, `TurnEngine` should:

- compute a deterministic approval request ID from session/turn/tool identity
- persist or reuse the pending request
- persist approval lifecycle events in the session log
- return a structured approval requirement instead of only a free-form denial string

This preserves idempotency and gives the turn loop enough information to render a truthful operator
message with the request ID.

### 3. Narrow approval tool surface

Add three app tools:

- `approval_requests_list`
- `approval_request_status`
- `approval_request_resolve`

These tools must follow the same visibility rules as the existing session tools, so operators can
inspect or resolve only requests that are visible from the current session lineage.

`approval_request_resolve` must support:

- `approve_once`
- `approve_always`
- `deny`

### 4. Automatic replay after approval

`approval_request_resolve` should be wired through runtime support so the original blocked tool call
can be resumed without asking the model to regenerate it.

Expected replay flow:

- `pending -> approved`
- optionally persist a runtime grant for `approve_always`
- `approved -> executing`
- resume the original tool request with an approval bypass scoped to that request or grant
- `executing -> executed` on success
- keep explicit failure evidence and `last_error` on resumed execution failure

### 5. Canonical attention model

Attention should be derived, not independently persisted.

Each approval request should expose:

- `execution_integrity`
- `grant_review`
- `grant_attention`
- `attention`

The canonical `attention` view is a union of source-tagged signals from:

- execution-side lifecycle/integrity state
- grant-side durability/review state

The first implementation does not need the entire historical filter matrix from old PR `#129`.
It should provide the parts that directly solve issue `#128`:

- per-request canonical attention view
- explicit execution/grant source tagging
- unified list-level attention summaries
- hotspot summaries by reason/action/tool/session
- explicit grant-side filters alongside existing status/session filters

## Attention Semantics

### Execution-side attention

Execution-side attention should cover:

- resumed execution failed
- resumed execution is stuck or incomplete
- replay outcome indicates follow-up inspection is needed

### Grant-side attention

Grant-side attention should cover:

- `approve_always` should have produced a durable grant but no grant exists
- durable grant exists but review age is stale/overdue

### Canonical union

List and status responses should expose both the source-specific structures and a merged
`attention` block containing:

- `needs_attention`
- `sources`
- `signals`
- `highest_escalation_level`

This preserves triage value without hiding whether the problem is execution-side or grant-side.

## Scope Boundaries

Included in this slice:

- durable approval requests
- durable runtime grants
- query/status/resolve approval tools
- automatic replay after approval
- canonical attention view and useful summaries/filters
- tests and docs updates needed to support the new lifecycle

Not included in this slice:

- cross-kernel approval runtime unification
- UI/channel-native approval workflows
- config file rewrites for `approve_always`
- speculative global grants outside current session lineage

## Validation Strategy

The work should land in explicit red-green slices:

1. repository schema and lifecycle tests
2. turn-engine approval request materialization tests
3. approval tool query tests
4. approval resolve and replay tests
5. attention summary/filter tests
6. formatting, clippy, and full workspace tests

The final GitHub delivery must link the replacement PR to issue `#128` and treat closed PR `#129`
as superseded by a fresh reconstruction rather than a revived branch.
