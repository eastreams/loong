# CLI Session Latest Selector Design

## Goal

Add a minimal `--session latest` selector to `loong ask` and `loong chat` so the CLI can resume
the most recent meaningful root conversation without requiring the operator to remember a concrete
session id.

## Current Repo Facts

- `crates/app/src/chat.rs`
  - `initialize_cli_turn_runtime_with_loaded_config()` resolves the session id before the CLI
    runtime is fully constructed
  - `resolve_cli_session_id()` currently treats any non-empty `--session` value as a literal
    session id
- `crates/app/src/session/repository.rs`
  - `SessionSummaryRecord` already exposes the metadata needed for selection:
    - `kind`
    - `updated_at`
    - `archived_at`
    - `turn_count`
  - `load_session_summary_with_legacy_fallback()` already preserves legacy turn-only sessions for
    single-session lookups
  - `list_visible_sessions()` already sorts session summaries by `updated_at DESC, session_id ASC`
- `crates/daemon/src/lib.rs`
  - `Ask` and `Chat` already parse `--session` as an optional string
  - CLI parsing does not need a new flag to support selector semantics

## Problem

The CLI supports only two session-selection modes today:

- omit `--session` and fall back to the implicit `default` session
- pass an explicit concrete session id

That leaves a real continuity workflow uncovered:

- resume the latest meaningful root conversation

Without that selector, operators must either remember ids, inspect the session store manually, or
reuse `default` even when that is not the conversation they actually want.

## Constraints

- keep the change local to the existing `--session` flag
- do not add a generic selector language in the first iteration
- preserve current behavior for omitted `--session`
- preserve current behavior for explicit concrete session ids
- avoid hardcoding session ids or out-of-band state
- keep legacy turn-only root sessions eligible when possible

## Options Considered

### Option 1: Interpret `latest` inside `resolve_cli_session_id()` with direct repository access

Why not:

- `resolve_cli_session_id()` currently has no config or memory context
- adding filesystem/database discovery directly into that narrow helper would blur responsibilities
- it would make testing and future selector growth harder

### Option 2: Resolve session selectors after config is loaded and memory is ready

This means:

- keep `--session` parsing unchanged
- add a selector-aware resolver in the CLI runtime initialization path
- let that resolver use `SessionRepository` and `MemoryRuntimeConfig`

Why this is the recommended option:

- smallest correct structural change
- reuses the existing session repository instead of inventing a parallel lookup path
- keeps selector behavior scoped to the CLI runtime layer
- preserves the literal-session-id fast path

### Option 3: Add a separate `--latest-session` flag

Why not:

- duplicates selection concepts across flags
- complicates CLI precedence rules
- provides less extensibility than keeping selection under the existing `--session` surface

## Recommended Design

Treat `latest` as a reserved selector value only for the CLI ask/chat session-resolution path.

Behavior:

1. trim the incoming `session_hint`
2. if no hint is provided:
   - keep the current implicit `default` behavior for normal ask/chat
   - keep the current explicit-session requirement for concurrent CLI host
3. if the hint is not `latest`:
   - keep returning it as a literal session id
4. if the hint is `latest`:
   - open the configured session repository
   - resolve the latest resumable root session
   - return its concrete session id
   - fail with a clear error if no eligible session exists

Minimal selection rule:

- session kind must be `root`
- session must not be archived
- session must have at least one persisted turn
- newest `updated_at` wins
- `session_id ASC` remains the stable tie-breaker
- legacy turn-only root sessions remain eligible

## Legacy Compatibility Decision

`latest` should include legacy turn-only root sessions.

Reason:

- the repository already preserves legacy session continuity for targeted summary lookups
- excluding those sessions would silently skip real conversation history after upgrade
- this can be implemented cleanly in one repository helper instead of spreading fallback logic

## Why This Is The Smallest Correct Fix

- no new CLI flags
- no new daemon command family
- no generic selector DSL
- no repo/worktree identity persistence in this iteration
- no change to existing explicit session id behavior

## Testing Strategy

Add red-green coverage for:

- repository selection:
  - newest eligible root session is selected
  - delegate-child sessions are excluded
  - archived sessions are excluded
  - empty sessions are excluded
  - legacy turn-only root sessions remain eligible
- app runtime selection:
  - `latest` resolves to the expected concrete session id during CLI runtime initialization
  - missing eligible sessions returns a clear error
  - omitted `--session` still uses `default`
  - explicit concrete session ids still pass through unchanged
- daemon CLI parsing:
  - `--session latest` is accepted for `chat`
  - `--session latest` is accepted for `ask`

## Scope Boundary

This PR will not:

- add repo-scoped or worktree-scoped selection
- add selectors beyond `latest`
- add new session-history or session-inspection CLI commands
- modify the separate daemon sessions shell work
