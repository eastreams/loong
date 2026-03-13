# Memory Search Design

## Context

LoongClaw's current thick app-layer tool surface is centered on session inspection and delegation:

- `sessions_list`
- `sessions_history`
- `session_status`
- `session_events`
- `session_archive`
- `session_unarchive`
- `session_cancel`
- `session_recover`
- `session_wait`
- `sessions_send`
- `delegate`
- `delegate_async`

That surface is now substantially stronger, but it still lacks a truthful memory retrieval primitive.
The app already persists transcript turns into SQLite and already has session visibility rules, yet
the only memory-facing behavior exposed to the model is indirect session history inspection.

External comparison points in the same direction, but also clarifies what *not* to do:

- OpenClaw's `memory_search` rides on a much heavier substrate with indexed Markdown memories,
  FTS, embeddings, and hybrid retrieval.
- ZeroClaw's memory recall similarly depends on FTS, vector search, hybrid merge, and pluggable
  memory backends.
- NanoBot and NanoClaw both reinforce that memory and scheduling are useful, but their broader
  runtime architecture is substantially heavier than LoongClaw's current app substrate.

The correct next step for LoongClaw is not "semantic memory." It is a truthful transcript-backed
search primitive over the durable data the app already owns.

## Problem

Operators and models can inspect a known session's history, but they cannot ask the system to find
where a fact, phrase, or prior instruction appeared within the caller's visible transcript scope.

Today that means:

- history can only be inspected session-by-session
- agents must already know which session to inspect
- archived but still relevant sessions are harder to rediscover
- transcript persistence exists, but there is no direct search affordance over it

This is a real product gap and a better target than broadening into scheduler, browser, or fake
semantic-memory claims.

## Goals

- Add a truthful `memory_search` app tool for transcript retrieval.
- Search only over persisted transcript turns in SQLite `turns`.
- Reuse existing session visibility rules instead of broadening authority.
- Support single-target, batch-target, and default visible-scope searches.
- Return enough structure for agents to identify where a hit came from without dumping full
  transcript bodies.
- Keep the tool contract honest about match semantics in this phase.

## Non-Goals

- No embeddings, vector search, BM25, hybrid recall, or semantic ranking.
- No search over `session_events` or other control-plane records.
- No new long-term memory store distinct from transcript persistence.
- No full-content export behavior; complete transcript reading remains `sessions_history`.
- No pagination or streaming in the first slice.
- No scheduler, cron, browser, or web retrieval work in this phase.

## Chosen Primitive

Add `memory_search`.

`memory_search` searches durable transcript turns that are visible from the calling session and
returns structured matches. It is intentionally transcript search, not knowledge-base search and
not semantic recall.

The tool should be described truthfully:

- source: transcript turns already written to SQLite
- match type: exact text / substring-style search
- authority: existing visible session scope only
- output: match records with snippets and identifiers, not synthesized answers

## Scope Rules

### Visible scope

When the request does not specify a target session, `memory_search` searches the caller's current
visible scope:

- the current root session plus visible descendant delegate sessions
- or only `self` when session visibility is configured that way

### Single-target scope

When the request specifies `session_id`, the tool searches only that target session.

Rules:

- target must be visible under the existing visibility model
- target may be archived; archive affects inventory visibility, not transcript existence
- legacy current-session transcript rows remain eligible through the same best-effort fallback used
  elsewhere in session inspection

### Batch scope

When the request specifies `session_ids`, the tool searches the visible subset of those targets.

Rules:

- visible targets are searched
- hidden or missing targets are reported in structured skipped metadata
- one bad target must not fail the entire batch request

## Request Contract

`memory_search` accepts these fields:

- `query` (required)
- `session_id` (optional, mutually exclusive with `session_ids`)
- `session_ids` (optional, mutually exclusive with `session_id`)
- `limit` (optional, default `20`, max `100`)
- `excerpt_chars` (optional, default `120`, clamped to a safe range)

Request rules:

- `query` must be a non-empty string after trimming
- `session_id` and `session_ids` cannot both be present
- no explicit target means "search visible scope"
- single-target invisible requests fail like existing direct inspection tools
- batch requests use best-effort result accounting instead of failing on partial invisibility

Representative requests:

```json
{
  "query": "timeout budget"
}
```

```json
{
  "query": "handoff note",
  "session_id": "delegate:child-123",
  "limit": 10,
  "excerpt_chars": 160
}
```

```json
{
  "query": "memory adapter",
  "session_ids": ["root-a", "delegate:child-1", "hidden-root"],
  "limit": 25
}
```

## Response Contract

The top-level response should stay simple and explicit:

- `query`
- `scope`
- `matches`
- `returned_count`
- `limit`
- `truncated`

### Scope payload

`scope` should include:

- `mode`: `visible`, `single`, or `batch`
- `current_session_id`
- `searched_session_ids`
- `searched_session_count`
- `skipped_targets` for batch requests

`skipped_targets` should classify each skipped target, for example:

- `skipped_not_visible`
- `skipped_not_found`

### Match payload

Each match should include:

- `session_id`
- `turn_id`
- `role`
- `ts`
- `content_snippet`
- `match`

`match` should include:

- `query`
- `match_kind` with phase-one value `substring`
- `excerpt_chars`

Representative match shape:

```json
{
  "session_id": "delegate:child-123",
  "turn_id": 91,
  "role": "assistant",
  "ts": 1763020000,
  "content_snippet": "...timeout budget was reduced after the second retry spike...",
  "match": {
    "query": "timeout budget",
    "match_kind": "substring",
    "excerpt_chars": 120
  }
}
```

## Query Semantics

Phase one should deliberately use the simplest honest query semantics:

- search only `turns.content`
- perform substring-style text matching
- order hits by most recent first: `ts DESC`, then `id DESC`

This is intentionally not positioned as "best" or "most relevant" retrieval. It is recent-first
transcript search over exact stored text.

## Why Not FTS Or Semantic Memory Yet

LoongClaw does not yet have the supporting substrate required to make stronger claims:

- no memory-specific chunking pipeline
- no embedding lifecycle
- no vector index maintenance
- no hybrid merge semantics
- no durable semantic-retrieval contract already exposed elsewhere in the app

Adding a tool named `memory_search` that clearly performs transcript text search is still honest.
Adding a tool that implies semantic recall without those supporting layers would not be.

This design leaves a clean future path:

- phase 1: `LIKE` / substring transcript search
- future phase: SQLite FTS5 keyword search
- later phase: hybrid or semantic retrieval only once indexing and consistency semantics are real

## Internal Design

### Storage and query layer

Add a transcript search helper on top of the existing SQLite memory store.

The query should:

- read from `turns`
- restrict to resolved target session ids
- match on `content`
- return `id`, `session_id`, `role`, `content`, and `ts`

The helper should not search `session_events`, because control-plane events are intentionally kept
out of transcript history and should stay out of transcript search results too.

### Visibility layer

Reuse the existing session repository and visibility semantics:

- `list_visible_sessions`
- `is_session_visible`
- legacy current-session fallback where already supported

This ensures `memory_search` behaves like the session tool family rather than introducing a second
visibility model.

### Tool layer

Expose `memory_search` as a real app tool:

- add it to the tool catalog and provider definitions
- gate it under session-tool enablement for this phase
- implement it in a dedicated tool module rather than expanding `session.rs` further

This keeps the new tool aligned with the thick app-layer direction while containing code growth.

## Error Model

Representative direct errors:

- `memory_search_invalid_request: ...`
- `visibility_denied: ...`
- `session_not_found: ...`

Batch requests should not fail for hidden or missing targets. Those targets should instead appear
under `scope.skipped_targets`.

No matches is not an error. It should return:

- `matches: []`
- `returned_count: 0`
- `truncated: false`

## Testing Plan

Minimum required coverage:

- single-session search returns structured hits with `turn_id`, `role`, `ts`, and snippet
- default visible-scope search includes current root plus visible descendant delegate sessions
- delegated child search cannot reach hidden or sibling sessions outside visibility rules
- archived visible sessions still contribute searchable transcript hits
- legacy current-session transcript rows can be searched without backfilling `sessions`
- single-target hidden session returns `visibility_denied`
- batch search returns mixed searched and skipped target accounting
- search results never include control-plane session events
- `limit` and `excerpt_chars` clamping works as documented
- no-hit queries return an empty result set rather than an error

## Alternatives Considered

### 1. Expose only `memory_window` or `memory_get`

Pros:

- smaller implementation
- almost no new query semantics

Cons:

- does not solve rediscovery
- overlaps heavily with `sessions_history`
- weaker user value than true transcript search

Rejected because the next thick tool should increase retrieval power, not just add another way to
read already-known history.

### 2. Build scheduler or cron tools first

Pros:

- externally comparable repos often expose scheduling primitives

Cons:

- LoongClaw does not yet have a truthful durable scheduler substrate
- delivery semantics, retries, ownership, and restart behavior are still undefined

Rejected because it would force a wider and less honest control surface than the app currently
supports.

### 3. Jump directly to semantic memory

Pros:

- richer retrieval story

Cons:

- requires a much heavier substrate than LoongClaw currently has
- would encourage product claims the runtime cannot yet back up

Rejected because the product direction is "few but thick and truthful," not "broad but suggestive."

## Why This Design

`memory_search` is the strongest next tool LoongClaw can add without pretending to have a larger
memory system than it does.

It deepens real substrate that already exists:

- durable transcript persistence
- session visibility rules
- session archive lifecycle
- app-layer orchestration tools

And it does so without widening into adjacent surfaces that still lack honest runtime backing.
