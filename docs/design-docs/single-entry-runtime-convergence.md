# Single-Entry Runtime Convergence

## Status

Active

## Summary

LoongClaw should continue to present one product entrypoint, `loong`, while making its
runtime boundaries more explicit inside the existing 7-crate workspace.

This document defines the first refactor lane:

- keep the current 7-crate DAG intact
- separate **session core** from **memory augmentation** semantically before any crate split
- converge turn-bearing hosts on shared runtime seams before introducing new crates
- avoid speculative surface crates such as a shared UI core until real reuse exists

## Why This Exists

The current repository already has a strong lower-layer shape:

- kernel governance remains explicit in [ARCHITECTURE.md](../../ARCHITECTURE.md)
- the 7-crate DAG is a stated non-negotiable in
  [Core Beliefs](core-beliefs.md) and
  [ARCHITECTURE.md](../../ARCHITECTURE.md)

The pressure is above that layer:

- [crates/app/src/lib.rs](../../crates/app/src/lib.rs) exposes provider, conversation, memory,
  session, chat, presentation, and TUI concerns from one crate
- [crates/app/src/chat.rs](../../crates/app/src/chat.rs) mixes CLI interaction, session
  selection, runtime initialization, and presentation
- [crates/app/src/memory/mod.rs](../../crates/app/src/memory/mod.rs) currently mixes transcript
  CRUD with durable recall / memory-system orchestration
- [crates/app/src/session/mod.rs](../../crates/app/src/session/mod.rs) depends on
  memory runtime config, which makes session durability look like a memory add-on
- [crates/daemon/src/gateway/api_turn.rs](../../crates/daemon/src/gateway/api_turn.rs) and
  [crates/daemon/src/control_plane_server.rs](../../crates/daemon/src/control_plane_server.rs)
  both need to stay aligned with the shared
  [AgentRuntime](../../crates/app/src/agent_runtime.rs) turn entry seam

The result is not that the kernel is unclear. The result is that product/runtime seams are still
too implicit.

## Constraints

The first refactor phase must preserve all of the following:

1. The 7-crate DAG remains the repository contract for now.
2. No new public product split. The user-facing entry remains `loong`.
3. No breaking changes to existing external CLI or protocol behavior.
4. Kernel-first routing and policy boundaries remain intact.
5. No new dependency is introduced solely for refactor convenience.

## Core Decisions

### 1. Single entry is a product decision, not a layering constraint

`loong` remains the only product entrypoint.

That does not imply that chat, runtime, session durability, ACP dispatch, TUI rendering, and
future Web/App surfaces should continue to share one internal ownership boundary.

The coding-agent runtime remains the one core execution substrate.

Non-coding experiences should be modeled as modular capability layers on that
same base rather than as a second runtime family with separate turn semantics,
tool semantics, or recovery semantics.

### 2. Session durability is core runtime state

LoongClaw must treat the following as runtime/session core, not optional memory:

- thread/session/transcript persistence
- recent window reads
- history replay and recovery
- compaction inputs and session lineage

The following remain memory augmentation:

- durable recall
- cross-session/project memory
- workspace memory documents
- recall systems and memory orchestration policies

The current repository does not yet enforce that split cleanly, so the first job is semantic
ownership, not immediate crate extraction.

### 3. Host turn convergence comes before crate extraction

Turn-bearing hosts should stop re-implementing runtime preparation logic independently.

In practice, the first convergence target is host-submitted agent turns:

- gateway `/v1/turn`
- control-plane `/turn/submit`
- future host/runtime surfaces that submit ACP-backed turns

Phase 0 should converge those hosts on one runtime-facing entry seam rather than letting each
daemon surface hand-roll turn bootstrap rules.

### 4. Physical crate extraction is a later step

Potential future crates such as `sessions`, `runtime`, `gateway`, or `tui` are architectural
directions, not immediate refactor obligations.

The repository should only split crates after:

- ownership boundaries are stable inside the current crates
- host call sites already converge on shared seams
- tests prove the seams are behavior-preserving

### 5. Not every remaining `memory::*` call is refactor debt

After the session-store convergence work, the remaining `memory::*` references
fall into three buckets:

- the thin [session::store](../../crates/app/src/session/store.rs) adapter itself
- true memory-augmentation paths such as staged envelope hydration / recall
- memory-facing tools and their tests

Those should not be collapsed into `session::store` just to reduce grep hits.
The goal is semantic clarity, not zero textual mentions of `memory`.

## Phase Plan

### Phase 0: Runtime convergence inside the existing DAG

Goal:

- remove duplicated host turn bootstrap logic
- keep daemon hosts thin and behaviorally identical
- document the intended seam in-repo

Implementation shape:

- shared host turn entry through [agent_runtime.rs](../../crates/app/src/agent_runtime.rs)
- gateway and control-plane consume that seam instead of bespoke bootstrap code
- no new crate introduced

### Phase 1: Session vs memory semantic split inside `app`

Goal:

- make transcript/window/session durability read as session runtime state
- make durable recall read as augmentation

This phase may still live inside `crates/app` while module ownership changes.

### Phase 2: Host/runtime seam hardening

Goal:

- make host call sites depend on shared runtime entry helpers rather than bespoke assembly
- keep CLI/TUI behavior intact while reducing cross-module knowledge in daemon surfaces

### Phase 3: Re-evaluate physical crate extraction

Only after phases 0-2 are stable should the repository decide whether new crates are justified.

## Non-Goals

The first refactor phase does **not** do the following:

- split the workspace beyond the current 7 crates
- create a generic shared UI core
- rename the product into separate `code` and `agent` surfaces
- redesign protocol contracts for speculative future clients

It also does **not** rebrand true memory-augmentation surfaces as session-core
surfaces. Session transcript durability and memory recall should become clearer
by moving apart, not by forcing them through one namespace.

## Acceptance Criteria

The convergence plan is successful when all of the following are true:

1. Gateway and control-plane turn submission reuse the same runtime-facing entry seam.
2. Host bootstrap logic lives in the runtime-facing `app` layer rather than daemon-only helpers.
3. Existing external request/response behavior remains unchanged.
4. Tests cover the shared seam and prevent host-specific duplicate bootstrap logic from drifting.
5. Repository docs clearly state that session durability and memory augmentation are distinct
   concerns even before any crate split.
