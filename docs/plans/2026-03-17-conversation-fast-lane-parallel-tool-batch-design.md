# Conversation Fast-Lane Parallel Tool Batch Execution Design

Date: 2026-03-17
Branch: `issue-269-parallel-tool-batch-fast-lane`
Issue: `#269`
Scope: add a constrained, fail-closed parallel tool batch executor to the
conversation fast lane without changing safe-lane plan execution

## Problem

`alpha-test` already models provider output as a `ProviderTurn` containing
`Vec<ToolIntent>`, and provider shaping already extracts multiple tool intents
from OpenAI, Anthropic, Bedrock, and responses-style payloads. The runtime,
however, still executes those intents as a serial short-circuit loop in
`TurnEngine`.

That leaves three structural problems:

1. multi-tool read-heavy turns pay avoidable latency because eligible calls
   cannot run concurrently
2. the runtime has no explicit batch contract for preflight, blocking
   conditions, approval barriers, and result ordering
3. the current serial loop can partially execute early intents before a later
   approval or binding failure terminates the turn

The third point matters as much as latency. A runtime that claims multi-tool
support but lacks an explicit batch barrier invites ambiguous partial side
effects.

## Goals

1. Add an explicit batch execution contract to the fast-lane conversation
   runtime.
2. Fail the whole batch before execution if any preflight condition blocks the
   turn.
3. Execute explicitly safe tools concurrently when the feature is enabled.
4. Preserve assistant source-order output replay so follow-up payload handling
   and discovery-first lease parsing remain stable.
5. Keep the first iteration small, reviewable, and easy to disable.

## Non-goals

1. Do not redesign safe-lane plan execution in this slice.
2. Do not parallelize governed approval flows or topology-mutating tools.
3. Do not change the existing tool-result envelope format.
4. Do not transplant the `spec/programmatic` scheduler implementation into
   `app`.
5. Do not widen `TurnEngine` into a persistence-aware conversation coordinator
   in this slice.

## Alternatives Considered

### A. Keep the current executor and only raise tool-step limits

Rejected. That permits more intents per turn, but it does not improve latency
or solve the missing batch contract. It also leaves the partial-execution
ambiguity untouched.

### B. Add explicit batch phases plus per-tool scheduling metadata

Recommended. This keeps the change local to the conversation fast lane while
making the runtime semantics precise:

1. sequential preflight across the batch
2. fail-closed barrier before execution
3. constrained concurrent execution for safe tools only
4. source-order finalization

### C. Reuse the `spec/programmatic` scheduler directly

Rejected for this slice. The scheduler ideas are good and should inform the
design, but pulling that implementation into `app` now would add too much scope
and blur the boundary between spec orchestration and conversation runtime.

## Decision

Implement option B with a deliberately small contract:

1. add fast-lane config gates for parallel execution
2. add explicit scheduling metadata to the tool catalog
3. refactor `TurnEngine` into `prepare -> execute -> finalize` phases
4. run batches in parallel only when:
   - the feature flag is enabled
   - the batch has more than one intent
   - every prepared intent is marked `ParallelSafe`
5. fall back to prepared sequential execution for mixed or serial-only batches
6. leave safe-lane plan execution unchanged

## Proposed Design

### 1. Constrained config surface

Add two fast-lane-only config fields:

1. `conversation.fast_lane_parallel_tool_execution_enabled = false`
2. `conversation.fast_lane_parallel_tool_execution_max_in_flight = 4`

These are intentionally separate from `turn_loop.max_tool_steps_per_round` and
from safe-lane settings. The feature should remain opt-in and should not imply
that the entire conversation runtime is now parallel-aware.

Operators will still need to raise `fast_lane_max_tool_steps_per_turn` above `1`
before any multi-tool fast-lane turn can execute.

### 2. Tool-level scheduling metadata

Add a small scheduling enum to the tool catalog:

1. `SerialOnly`
2. `ParallelSafe`

This is intentionally smaller than a future resource-aware scheduler taxonomy.
The first iteration only needs to distinguish "never batch in parallel" from
"safe to parallelize in the fast lane".

Initial `ParallelSafe` tools should stay narrow and read-oriented:

1. `file.read`
2. `tool.search`
3. `web.fetch`
4. `sessions_list`

`sessions_list` is included because it is a read-only app tool that gives the
test suite a controlled fast-lane app-path target without introducing runtime
test hooks. Everything else remains `SerialOnly`.

### 3. Explicit batch phases in `TurnEngine`

Refactor `execute_turn_in_context(...)` around three internal phases.

#### Phase A: prepare the whole batch

For each intent, in assistant source order:

1. resolve the tool and validate visibility
2. inject ingress payload and browser/session scope augmentation
3. unwrap `tool.invoke` into an inner app-tool request when needed
4. determine the effective execution kind
5. check kernel binding requirements for core tools
6. run governed approval preflight for app tools
7. derive the scheduling class from the resolved descriptor

If any intent returns one of these blockers, the batch stops before execution:

1. `tool_not_found`
2. `tool_not_visible`
3. `no_kernel_context`
4. `NeedsApproval`
5. policy denial
6. descriptor/preflight failure

That turns the current serial short-circuit loop into a true batch barrier.

#### Phase B: execute prepared intents

Execution uses one of two paths:

1. prepared sequential path
2. prepared parallel path

The parallel path only activates when all prepared intents are
`ParallelSafe`. Mixed batches fall back to sequential execution for the whole
prepared list. That keeps the first implementation simple and predictable.

Parallel execution uses bounded concurrency via `max_in_flight`, but final
result collection is reconstructed in source order.

#### Phase C: finalize in source order

Formatting stays exactly the same as today:

`[ok] {...}` or `[error] {...}` lines joined with `\n` in assistant source
order.

This is a compatibility contract, not an implementation detail. Follow-up
payload reduction and discovery-first lease extraction already assume they can
scan line-by-line while preserving assistant order.

### 4. Failure semantics

The batch contract for this slice is:

1. preflight blockers fail closed before execution
2. execution failures during the sequential path terminate immediately
3. execution failures during the parallel path are collected, then surfaced in
   source order using the existing `TurnResult` mapping

Because phase 1 only parallelizes low-risk read-oriented tools, execution-time
partial success is acceptable and bounded. We are not parallelizing mutating or
approval-gated tools in this patch.

### 5. Scope boundary on persistence

`persist_tool_decision(...)` and `persist_tool_outcome(...)` already exist, but
wiring them cleanly would require widening `TurnEngine`'s contract or adding a
new batch report surface through the coordinator. That is a real follow-up, but
it is intentionally out of scope here to keep this patch local to fast-lane
execution semantics and regression coverage.

## Expected Behavioral Outcome

1. fast-lane turns can execute eligible multi-tool batches concurrently when
   explicitly enabled
2. a later approval or binding blocker no longer allows an earlier intent to
   run first
3. output ordering remains deterministic and source-aligned
4. follow-up payload compaction and discovery-first lease parsing continue to
   work with multi-line tool-result text
5. safe-lane plan execution keeps its current behavior in this slice

## Test Strategy

Add focused regression coverage for:

1. parallel-safe app-tool batches execute concurrently while preserving
   source-order output
2. a mixed batch with a later approval requirement fails before any earlier
   intent executes
3. a mixed batch with a later `no_kernel_context` blocker fails before any
   earlier app intent executes
4. config defaults and TOML overrides cover the new fast-lane parallel fields
5. discovery-first lease parsing remains stable for multi-line tool-result
   content in source order

## Why This Slice Matters

This is the smallest change that makes the conversation runtime honest about
multi-tool execution. It closes a real semantics hole, improves latency for a
safe subset of turns, and avoids dragging safe-lane or cross-crate scheduling
unification into the same PR.
