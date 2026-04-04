# Tasks CLI Latest Session Selector Design

## Goal

Add a minimal `--session latest` selector to the `loongclaw tasks` command family so operators can
manage background tasks against the newest resumable root session without looking up a concrete
session id first.

## Tracking

- GitHub issue: `#800`

## Current Repo Facts

- `crates/daemon/src/tasks_cli.rs`
  - `execute_tasks_command()` loads config and memory runtime state before it normalizes the
    session scope.
  - `normalize_session_scope()` currently trims the raw string and rejects empty input.
  - Every `tasks` subcommand uses the same resolved `current_session_id`.
- `crates/app/src/session/repository.rs`
  - `SessionRepository::latest_resumable_root_session_summary()` already returns the newest
    resumable root session.
  - That repository method already excludes archived sessions, excludes non-root sessions, and
    requires persisted turns.
- `crates/app/src/chat.rs`
  - `ask` and `chat` already resolve `--session latest` after memory config is available.
  - The selector semantics already exist in the product as an operator-visible behavior.
- `crates/daemon/tests/integration/tasks_cli.rs`
  - Existing integration tests already cover literal session ids across `create`, `list`, `status`,
    `events`, `wait`, `cancel`, and `recover`.
  - The best missing coverage is selector resolution at the shared `execute_tasks_command()` entry
    point.

## Problem

The `tasks` CLI accepts `--session`, but it still treats every non-empty value as a literal session
id. That creates an inconsistency:

- `ask` and `chat` can resume the latest resumable root with `--session latest`
- `tasks` still requires a remembered root session id

That inconsistency matters most when an operator wants to inspect or manage delegate work for the
conversation they were just using.

## Constraints

- keep the existing `tasks --session` flag surface unchanged
- preserve the current requirement that `tasks` needs a non-empty session scope
- preserve literal session ids unchanged
- reuse existing repository selection semantics instead of duplicating SQL or filter rules
- avoid widening scope to unrelated `--session` consumers in the same PR

## Options Considered

### Option 1: Add a local selector-aware resolver in `tasks_cli.rs`

This means:

- keep `normalize_session_scope()` for trim and empty-check behavior
- add a thin resolver that maps `latest` to the repository helper
- keep all selector handling inside the `tasks` entry point

Why this is the recommended option:

- smallest correct change
- no new public abstraction
- reuses the repository as the single source of selector truth
- keeps the patch scoped exactly to the command family that is missing the behavior

### Option 2: Extract a new shared CLI session-selector helper

Why not now:

- `chat` and `tasks` have different session requirements around implicit defaults
- the repository already contains the hard part of the selection logic
- adding a broader helper now would increase coupling without reducing meaningful complexity

### Option 3: Broaden `latest` handling to every `--session` consumer

Why not now:

- not every consumer means “latest resumable root conversation”
- the user asked for a minimal, non-speculative improvement
- widening scope would add risk without proving additional value in this follow-up

## Recommended Design

Keep the selector behavior local to `tasks_cli.rs`, but reuse the existing repository method for the
actual selection semantics.

Behavior:

1. load config
2. initialize the runtime environment
3. build `MemoryRuntimeConfig`
4. trim the raw `--session` value
5. if the trimmed value is empty:
   - return the existing `tasks CLI requires a non-empty session scope` error
6. if the trimmed value is not `latest`:
   - keep returning it as the literal session id
7. if the trimmed value is `latest`:
   - open `SessionRepository`
   - call `latest_resumable_root_session_summary()`
   - return the concrete root session id
   - fail with a clear `latest`-specific error if no resumable root session exists

## Error Handling

- Empty input remains the same validation error that `tasks` already returns today.
- `latest` with no resumable root session should fail before subcommand execution begins.
- The no-match error should mention both `tasks` and `latest` so operators can see the root cause
  immediately.

## Testing Strategy

Add daemon integration coverage in `crates/daemon/tests/integration/tasks_cli.rs` for:

- success path:
  - create two resumable root sessions with distinct timestamps
  - run `tasks create --session latest`
  - assert that the resolved `current_session_id` is the newest root session
  - assert that the queued delegate child is attached to that selected root session
- failure path:
  - run a `tasks` command with `--session latest` and no resumable root sessions present
  - assert that the command fails with a clear `latest` selector error

Existing tasks integration tests already cover literal session ids across the subcommands, so this
PR only needs to prove the new selector resolution branch and its wiring.

## Scope Boundary

This PR will not:

- add a generic selector DSL
- add a new CLI flag
- change `ask` or `chat`
- change other session-aware CLIs
- change session repository semantics
