# Kernel Policy Unification Design

Date: 2026-03-15
Branch: `feat/issue-45-kernel-policy-unification`
Scope: kernel-first unification of tool-execution policy in `alpha-test`
Status: approved direction, implementation slice 1

## Problem

`alpha-test` already moved the dangerous shell/file policy decisions into the kernel path, but the
runtime contract is still softer than the architecture claims. The main remaining drift is not
"missing policy extensions"; it is "kernel context is still optional across hot execution paths."

Today the runtime still mixes three different concerns:

1. turn validation
2. kernel binding
3. tool execution

That mixing shows up in several places:

1. `TurnEngine::evaluate_turn(...)` treats "kernel required for execution" as a policy denial
   instead of a phase boundary.
2. `TurnEngine::execute_turn(...)` still accepts `Option<&KernelContext>`.
3. The safe-lane plan executor still threads `Option<&KernelContext>` into per-node tool
   execution.
4. Coordinator paths still treat "missing kernel" as a normal late failure case instead of
   resolving the execution contract before entering the tool runtime.

The result is that the kernel is conceptually authoritative, but operationally optional. That is
the wrong shape for a kernel-first runtime.

## Goals

1. Make tool execution a kernel-mandatory phase in both fast lane and safe lane.
2. Separate pure turn validation from kernel-bound execution.
3. Remove `Option<&KernelContext>` from the inner tool execution surfaces in this slice.
4. Keep current user-visible failure behavior stable where reasonable, especially
   `no_kernel_context` and existing policy/error classification.
5. Create a clean seam that later kernelizes memory, context engines, and provider-adjacent flows
   without reworking the turn contract again.

## Non-Goals

1. Do not remove every `Option<&KernelContext>` in the repository in one patch.
2. Do not redesign context-engine, provider, or persistence traits in this slice.
3. Do not retune lane routing, approval defaults, or safe-lane governor heuristics here.
4. Do not change audit schema or introduce durable audit persistence here.

## Current State

### What is already correct

1. `crates/app/src/tools/mod.rs` exposes a kernel-bound `execute_tool(...)` entrypoint that
   requires `&KernelContext`.
2. `crates/app/src/context.rs` now grants `FilesystemRead` and `FilesystemWrite`.
3. Shell/file policy extensions are registered into the kernel bootstrap path.
4. Approval policy defaults already exist in `crates/spec` and are not the missing primitive.

### What is still architecturally wrong

1. `crates/app/src/conversation/turn_engine.rs` still models "missing kernel" inside the execution
   engine.
2. `crates/app/src/conversation/turn_coordinator.rs` safe-lane node execution still unwraps an
   optional kernel per tool node.
3. Validation and execution are still coupled tightly enough that missing kernel context looks like
   a late execution failure rather than an unbound runtime.

## Approaches Considered

### A. Broad repository-wide kernel hardening now

Make `KernelContext` mandatory across conversation, provider, context engine, persistence, channel,
and tool surfaces in one breaking pass.

Pros:
- cleanest theoretical end state
- eliminates optional kernel context quickly

Cons:
- too much surface area for one reviewable change
- high regression risk in unrelated lanes
- hard to attribute failures

### B. Mandatory tool-execution kernel first

Treat tool execution as the first hard boundary:

1. validation stays pure
2. coordinator binds kernel once before entering tool execution
3. fast-lane and safe-lane tool execution paths require `&KernelContext`

Pros:
- directly fixes the highest-value control-plane gap
- small enough to review and verify
- creates the right contract for follow-on kernelization

Cons:
- leaves optional kernel context in surrounding traits for now
- requires transitional adapter logic at the coordinator boundary

### C. Pure helper extraction only

Keep the current signatures and just deduplicate some tool execution code.

Pros:
- smallest patch

Cons:
- does not change the contract
- keeps the kernel optional in the exact places that matter
- would look cleaner without actually being safer

## Decision

Implement Approach B.

The correct first architecture move is to make tool execution a kernel-mandatory runtime phase.
That is the narrowest slice that materially strengthens the kernel model without dragging every
conversation subsystem into the same patch.

## Target Design

### 1. Split turn validation from tool execution

`TurnEngine` should expose a pure validation phase that:

1. returns final text immediately when there are no tool intents
2. rejects unknown tools and step-budget violations
3. returns an explicit "tool execution required" result instead of inventing
   `kernel_context_required`

This keeps validation honest: it validates the turn, not the runtime wiring.

### 2. Make kernel binding explicit at the coordinator boundary

When a validated turn requires tool execution, the coordinator becomes responsible for one explicit
decision:

1. kernel available -> enter tool execution
2. kernel unavailable -> synthesize the existing `no_kernel_context` policy denial before any tool
   execution path begins

This is the correct place for the fallback because it is the actual runtime binding boundary.

### 3. Make inner tool executors require `&KernelContext`

In this slice the following inner surfaces should stop accepting optional kernel context:

1. `TurnEngine` tool-execution entrypoint
2. safe-lane per-node tool execution helper
3. safe-lane plan node executor state

That change is the real contract hardening: once execution starts, the kernel is mandatory.

### 4. Share the kernel-bound tool execution path

Fast lane and safe lane should not each rebuild kernel tool execution semantics ad hoc. This slice
should converge them on one shared execution shape:

1. known tool names only
2. `InvokeTool` capability set
3. kernel policy/error classification
4. shared tool-result formatting

The goal is not a giant abstraction; the goal is one authoritative path for tool execution
semantics.

### 5. Preserve failure vocabulary in slice 1

This patch should keep existing public/runtime-facing failure codes stable where possible:

1. `tool_not_found`
2. `max_tool_steps_exceeded`
3. `no_kernel_context`
4. `kernel_policy_denied`
5. `tool_execution_failed`
6. `kernel_execution_failed`

The architecture changes, but the observable failure taxonomy stays stable enough for existing
tests, runtime analytics, and user expectations.

## Why This Is Kernel-First

This design makes the kernel the only legal execution substrate for tool-calling turns. The app
runtime still decides when to enter the tool phase, but once it does, the execution contract is
hard-bound to `&KernelContext`.

That is closer to Codex's approval/sandbox model than to an operator-product orchestration model:

1. execution policy is a runtime primitive
2. missing authority is resolved before execution begins
3. the execution core does not carry shadow optionality

OpenClaw-style operator safety features can still exist above this layer, but they should compose
with the runtime kernel rather than substitute for it.

## Follow-On Phases

After this slice lands, the next kernelization phases should be:

1. context-engine trait split: kernel-required vs kernel-agnostic operations
2. provider/runtime telemetry paths: explicit policy-free runtime helpers versus kernel-bound
   operations
3. persistence/session-history paths: stop treating kernel access as ambient optional plumbing
4. channel entrypoints: bootstrap or inject kernel context earlier so hot paths do not branch late

## Testing Strategy

1. Add RED tests for the new validation/execution split in `turn_engine`:
   - no-tool turn validates to final text
   - known-tool turn validates to "execution required" instead of `kernel_context_required`
   - invalid tool still fails validation
2. Add RED tests for coordinator or safe-lane helpers to prove:
   - missing kernel is rejected before entering tool execution
   - safe-lane execution still surfaces `no_kernel_context` from the coordinator boundary
3. Keep existing tool execution result/error classification tests green.
4. Run targeted conversation tests first, then broader `loongclaw-app` verification.

## Acceptance Criteria

1. Inner tool execution surfaces no longer accept `Option<&KernelContext>`.
2. Turn validation no longer uses `kernel_context_required` as a pseudo-policy result.
3. Fast-lane and safe-lane tool execution both bind kernel authority before entering execution.
4. Existing failure-code mapping remains stable for runtime behavior.
5. `docs/SECURITY.md` is updated to describe the current kernel-bound tool execution model instead
   of the stale pre-unification description.
