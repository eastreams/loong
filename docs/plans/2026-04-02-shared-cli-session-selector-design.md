# Shared CLI Session Selector Design

Date: 2026-04-02
Issue: `#809`
PR: pending
Status: Proposed for the current task branch

## Problem

The CLI surface now supports `--session latest` in multiple places, but the resolution logic is
still split across separate ownership boundaries:

1. `crates/app/src/chat.rs` resolves `latest` for `ask` and `chat`
2. `crates/daemon/src/tasks_cli.rs` resolves `latest` again for `tasks`

Both paths currently call the same repository query, but they do so through duplicated local
logic. That duplication creates a long-term drift risk:

1. one surface can change selector semantics without the other
2. one surface can change the `latest` token constant without the other
3. repository contract changes can be patched in one place and missed in the other

The operator-visible feature works today, but the ownership boundary is still wrong.

## Goal

Create one shared CLI session-selector helper that owns the reusable `latest` lookup contract,
then route both `chat` and `tasks` through that helper without changing the surrounding
surface-specific rules.

## Non-Goals

1. no new session selector DSL
2. no change to implicit default-session behavior in `chat`
3. no change to `tasks` session normalization rules beyond using the shared helper
4. no change to repository selection semantics
5. no attempt to unify every session-hint code path into one large abstraction

## Approaches Considered

### A. Keep the current duplicated logic and add a comment

Pros:

1. smallest immediate diff

Cons:

1. leaves the root cause unchanged
2. still allows semantic drift across CLI surfaces
3. does not create a reusable ownership boundary for future selector work

### B. Add a small shared helper for `latest` resolution only

Pros:

1. fixes the actual duplication seam
2. keeps default-session and explicit-session policies local to each caller
3. minimizes change scope while still improving long-term maintainability
4. keeps the repository dependency in one reusable place

Cons:

1. adds one small shared session helper surface

### C. Fully unify all CLI session-hint handling behind a large generic resolver

Pros:

1. centralizes more logic in one place

Cons:

1. expands scope far beyond the actual duplication
2. couples unrelated policies such as implicit default handling and explicit-session enforcement
3. increases regression risk for behavior that is already correct today

## Decision

Choose approach B.

The smallest correct move is to introduce one shared helper in the app session layer that exposes:

1. the canonical `latest` selector token
2. one helper that resolves the newest resumable root session id from a `MemoryRuntimeConfig`

Callers will keep their own boundary-specific behavior:

1. `chat` will still decide when `latest` should resolve and when a literal session id should be
   preserved
2. `tasks` will still own its non-empty normalization and surface-specific error wording

This keeps the refactor narrow and directly addresses the actual root cause: duplicated lookup
logic across CLI surfaces.

## Architecture

Extend `crates/app/src/session/mod.rs` with one small shared selector helper surface.

That helper should:

1. stay behind the existing `memory-sqlite` feature gate
2. build a `SessionRepository` from the provided `MemoryRuntimeConfig`
3. reuse `latest_resumable_root_session_summary()`
4. return `Option<String>` so each caller can preserve its own error text and policy decisions

Then update callers:

1. `crates/app/src/chat.rs` stops owning the selector token constant and repository lookup
2. `crates/daemon/src/tasks_cli.rs` stops owning its own selector token constant and repository
   lookup

## Validation Strategy

Minimum required validation for this refactor:

1. add focused tests for the shared helper contract
2. keep the existing app-layer `latest` runtime tests green
3. keep the existing daemon `tasks` integration coverage green
4. run workspace verification so the refactor proves it did not shift behavior elsewhere
