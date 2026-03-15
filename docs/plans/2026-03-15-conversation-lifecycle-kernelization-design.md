# Conversation Lifecycle Kernelization Design

Date: 2026-03-15
Branch: `feat/issue-45-kernel-policy-unification`
Scope: phase 2 kernelization of conversation lifecycle runtime interfaces
Status: approved direction, implementation slice 2

## Problem

Phase 1 made tool execution kernel-mandatory, but the conversation runtime still carries optional
kernel plumbing through its lifecycle interfaces. The most important remaining drift is in
`ConversationRuntime` and `ConversationContextEngine`:

1. `bootstrap`
2. `ingest`
3. `after_turn`
4. `compact_context`
5. `prepare_subagent_spawn`
6. `on_subagent_ended`

Those methods are not pure helpers. They are lifecycle hooks with side effects, but they still
accept `Option<&KernelContext>`. That keeps the runtime contract ambiguous:

1. some lifecycle work is conceptually kernel-bound
2. call sites still pass `None` deep into the trait stack
3. the trait signatures imply that lifecycle execution without kernel is a normal first-class mode

That is the next architectural mismatch after phase 1.

## Goals

1. Make conversation lifecycle hooks explicitly kernel-bound.
2. Remove `Option<&KernelContext>` from lifecycle methods on `ConversationRuntime`.
3. Remove `Option<&KernelContext>` from lifecycle methods on `ConversationContextEngine`.
4. Move the "no kernel available" decision to the runtime binding boundary instead of the trait
   boundary.
5. Preserve existing direct fallback behavior for non-lifecycle paths that are not in scope yet:
   `build_context`, `build_messages`, `request_turn`, `request_completion`, and `persist_turn`.

## Non-Goals

1. Do not make provider request paths kernel-mandatory in this slice.
2. Do not make `persist_turn` kernel-mandatory in this slice.
3. Do not redesign default/legacy context assembly behavior in this slice.
4. Do not remove all `Option<&KernelContext>` usage from the repository in one patch.

## Current State

### What phase 1 already fixed

1. `TurnEngine` now validates turns separately from tool execution.
2. Inner fast-lane and safe-lane tool execution now requires `&KernelContext`.
3. Missing kernel authority is handled at the tool execution binding boundary.

### What still drifts

1. `DefaultConversationRuntime` still forwards `Option<&KernelContext>` through lifecycle hooks.
2. `persist_and_ingest_turn(...)` still treats ingestion as an optional-kernel trait behavior
   instead of an explicit bound action.
3. `maybe_compact_context(...)` and after-turn finalization still enter trait methods with
   optional kernel state.
4. lifecycle-oriented test doubles still exercise no-kernel delegation as if that were the desired
   stable contract.

## Approaches Considered

### A. Change lifecycle trait signatures directly

Make lifecycle methods on `ConversationRuntime` and `ConversationContextEngine` require
`&KernelContext`, then make call sites explicitly choose:

1. call the lifecycle hook with bound kernel
2. skip it when no kernel is available

Pros:
- strongest contract improvement for the smallest conceptual model
- removes optional kernel plumbing from the real lifecycle seam
- keeps the binding decision where it belongs: at the caller

Cons:
- requires touching tests and multiple runtime call sites
- intentionally changes no-kernel lifecycle semantics

### B. Add parallel kernel-bound traits

Keep current optional trait methods and add a second trait for kernel-bound lifecycle hooks.

Pros:
- smaller migration risk
- easier to layer incrementally

Cons:
- duplicates the contract
- keeps the old ambiguous trait alive
- encourages shadow optional paths to persist

### C. Add wrapper helpers only

Leave the signatures alone and just wrap lifecycle calls at a few coordinator sites.

Pros:
- smallest patch

Cons:
- does not change the underlying contract
- leaves ambiguity in every implementation and test double
- continues to teach the wrong architecture

## Decision

Implement Approach A.

The kernelization goal is to make the contract honest, not merely tidier. Lifecycle hooks are
kernel-bound actions, so the trait surface should say that directly.

## Target Design

### 1. Lifecycle methods become kernel-bound

The following methods should require `&KernelContext` on both runtime traits:

1. `bootstrap`
2. `ingest`
3. `after_turn`
4. `compact_context`
5. `prepare_subagent_spawn`
6. `on_subagent_ended`

This makes lifecycle hooks explicitly different from ambient runtime helpers.

### 2. The binding decision moves upward

Callers that currently hold `Option<&KernelContext>` should make the decision explicitly:

1. if kernel is present, invoke the lifecycle hook
2. if kernel is absent, skip the hook and keep the rest of the flow running

That creates one truthful boundary instead of pushing optionality down into every implementation.

### 3. Persistence keeps its direct fallback for now

`persist_turn(...)` remains out of scope for this slice because it still underpins direct-memory
conversation fallback. This slice only changes the extra lifecycle behavior that sits around core
persistence:

1. turn persistence still runs
2. lifecycle ingestion becomes kernel-bound
3. context compaction becomes kernel-bound
4. after-turn and subagent lifecycle notifications become kernel-bound

### 4. No-kernel lifecycle behavior becomes explicit skip semantics

This slice intentionally changes the no-kernel behavior from "delegate optional lifecycle methods"
to "skip kernel-bound lifecycle hooks." That means:

1. no-kernel persistence still stores turns through existing direct fallback
2. no-kernel ingestion does not run
3. no-kernel compaction does not run
4. no-kernel after-turn hooks do not run
5. no-kernel subagent lifecycle hooks do not run

That is the right architecture for a kernel-first runtime: absence of kernel authority is resolved
once, not smeared through the call graph.

### 5. Tests should prove the new contract

The test suite should stop asserting no-kernel lifecycle delegation. Instead it should prove:

1. kernel-bound lifecycle hooks still delegate correctly when kernel is present
2. no-kernel persistence still works
3. no-kernel lifecycle hooks are skipped at the caller boundary

## Why This Is The Right Phase 2

This slice keeps the scope tight but meaningful. It does not overreach into provider request
kernelization or memory fallback removal, yet it eliminates one of the main remaining places where
optional kernel context still shapes the architecture.

It also sets up phase 3 cleanly:

1. provider request paths can stay optional or become split explicitly
2. persistence can later be separated into kernel-bound and direct-fallback surfaces
3. context assembly can later be split into legacy/direct and kernel-bound implementations without
   lifecycle ambiguity

## Testing Strategy

1. Add RED tests for runtime lifecycle delegation with required kernel context.
2. Add RED tests proving no-kernel persistence no longer triggers ingestion/after-turn/compaction
   hooks.
3. Re-run targeted conversation tests first.
4. Re-run full `loongclaw-app` verification after the trait change.

## Acceptance Criteria

1. Lifecycle methods on `ConversationRuntime` and `ConversationContextEngine` no longer accept
   `Option<&KernelContext>`.
2. Runtime callers make skip-vs-run lifecycle decisions explicitly.
3. Direct persistence and context-build fallback behavior remains intact.
4. Tests cover the new kernel-bound lifecycle semantics.
5. PR #151 can be updated without expanding scope into provider or persistence contract redesign.
