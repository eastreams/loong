# Session Tool List Pagination Design

## Goal

Add stable pagination semantics to the `sessions_list` app tool so operators and future CLI
surfaces can page through visible session results instead of being limited to the first truncated
window.

## Current Repo Facts

- `crates/app/src/tools/session.rs`
  - `execute_sessions_list()` filters visible sessions and truncates the final vector by `limit`
  - returns `matched_count` and `returned_count`, but does not expose a way to fetch the next page
- `crates/app/src/tools/catalog.rs`
  - `sessions_list_definition()` advertises `limit`, `state`, `kind`, `parent_session_id`,
    `overdue_only`, `include_archived`, and `include_delegate_lifecycle`
  - tool hint metadata for `sessions_list` currently only mentions `limit` and `state`
- `src/utils/listSessionsImpl.ts` in the reference codebase exposes `limit` plus `offset` so
  callers can paginate across sorted session results without inventing new ad-hoc filters

## Problem

`sessions_list` currently has a one-page contract.

When the visible session set exceeds `limit`, the caller learns only that more matches exist
through `matched_count > returned_count`. There is no first-class way to request the next page
without narrowing filters or raising the limit.

That makes the tool awkward for:

- operator workflows that need to scan a large delegate tree
- future CLI wrappers that want deterministic paging
- automation that wants stable, repeated windowed reads instead of overfetching

## Constraints

- keep the change small and local to the existing `sessions_list` tool contract
- do not redesign sorting or session visibility semantics
- avoid adding new repository-layer pagination because the current app-tool path already filters in
  memory after visibility and delegate-lifecycle enrichment
- preserve backward compatibility for callers that omit pagination fields

## Options Considered

### Option 1: Add cursor-style pagination

This would add fields such as `after_session_id` or `next_cursor`.

Why not:

- current ordering is already deterministic for the in-memory list, so index-based paging is
  enough for the first iteration
- a cursor contract would require more state coupling between callers and the current sort order
- it adds more surface area than the tool currently needs

### Option 2: Add `offset` pagination to `sessions_list`

This means callers can request `limit` plus `offset`, and the tool returns a stable page slice plus
 enough metadata to know whether more results remain.

Why this is the recommended option:

- matches the reference repo's list-session ergonomics
- preserves the existing tool shape and sorting behavior
- requires only one new input field and one new response flag
- keeps future CLI wrapping simple

### Option 3: Expose a new dedicated pagination tool

This would create a second list tool with different paging behavior.

Why not:

- splits one concept across multiple tools
- duplicates filter semantics and future maintenance work
- violates the smallest-correct-change goal

## Recommended Design

Extend `sessions_list` with one optional request field:

- `offset: integer`

Behavior:

1. collect and filter visible sessions exactly as today
2. compute `matched_count` before pagination
3. skip `offset` sessions from the front of the filtered list
4. apply the existing `limit`
5. return:
   - `matched_count`
   - `returned_count`
   - `has_more`
   - `filters.offset`

Parsing rules:

- missing or invalid `offset` falls back to `0`
- negative values also fall back to `0`
- valid positive integers are clamped to `usize`

Documentation updates:

- add `offset` to the JSON schema in `sessions_list_definition()`
- add `offset` to the compact tool argument hint metadata so prompt-shaping surfaces know the field
  exists

## Why This Is The Smallest Correct Fix

- no repository query changes
- no new tool names
- no new session filtering semantics
- backward-compatible default behavior when `offset` is omitted

## Testing Strategy

Add red-green coverage for:

- payload parsing: `offset` defaults to `0`, accepts positive integers, and ignores invalid values
- tool behavior: `sessions_list` with `offset` returns the correct page, preserves
  `matched_count`, updates `returned_count`, and surfaces `has_more`
- catalog contract: `sessions_list` schema and hint metadata include `offset`

## Scope Boundary

This PR will not:

- add cursor-based pagination
- change session ordering
- add pagination to `sessions_history`, `session_events`, or other tools
- add or modify the separate daemon CLI sessions shell work
