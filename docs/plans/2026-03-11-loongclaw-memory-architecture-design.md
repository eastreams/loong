# LoongClaw Memory Architecture Design

Date: 2026-03-11
Status: Approved for implementation

## Summary

LoongClaw needs a memory architecture that is pluggable at the policy level
before it becomes pluggable at the storage-backend level. The current MVP
memory path is still SQLite-first and sliding-window-first:

- config only exposes `sqlite_path` and `sliding_window`
- provider/chat/channel code reads memory by directly calling SQLite helpers
- runtime behavior cannot distinguish between "recent context", "condensed
  session memory", and "durable identity/profile memory"

That is enough for a basic chat loop, but it is not enough for the product
direction the migration work needs:

- one-click migration from other claws
- inheritance of prior identity/tuning/preferences
- user-selectable memory behavior
- future support for multiple memory backends without rewriting product logic

The recommended direction is:

- expose `memory.profile` as the primary user-facing choice
- keep `memory.backend` as a secondary implementation detail
- derive an internal `memory.mode` from the selected profile
- centralize memory hydration behind one app-layer orchestrator
- reserve a durable `profile_note` lane for imported identity/tuning

## Product Goals

- Let users choose memory behavior without having to understand storage
  backends.
- Preserve backward compatibility with today's config and SQLite runtime.
- Create a native place to carry imported claw identity/preferences forward.
- Keep memory stable, deterministic, and performance-aware.
- Keep the safety model explicit:
  memory should enrich context, not silently bypass runtime policy.

## Non-Goals For This Slice

- No vector store rollout yet.
- No LLM-generated long-term summaries yet.
- No cross-session semantic retrieval yet.
- No full migration importer in the same patch.
- No breaking change to existing `sqlite_path` or `sliding_window`.

## Current State

Today LoongClaw memory exists in three partially separated layers:

1. Kernel/contracts already support a generic memory plane with core and
   extension adapters.
2. App runtime still treats memory as direct SQLite storage.
3. User config only knows about a single sliding-window policy.

This creates two problems:

1. The kernel is more abstract than the product runtime that sits above it.
2. Migration/import features have nowhere durable to store inherited identity
   outside of prompt text.

## Design Principle: Profile First, Backend Second

Users should not start by choosing `sqlite`, `redis`, or `vector`.

They should start by choosing the behavior they want:

- keep only recent context
- keep recent context plus condensed earlier context
- keep recent context plus a durable user/agent profile

That gives LoongClaw a stable product surface even while storage engines change
later.

## Approaches Considered

### Approach 1: Backend Abstraction Only

Add `memory.backend = sqlite|...` and keep all runtime behavior unchanged.

Pros:

- low implementation cost
- easy to explain internally

Cons:

- does not solve user-facing memory behavior
- does not help migration/nativeization
- leaves provider/chat code coupled to low-level retrieval details

### Approach 2: Profile/Mode Abstraction At App Layer

Keep SQLite as the only backend for now, but introduce explicit memory
profiles/modes and route all retrieval through a shared app-layer orchestrator.

Pros:

- matches the product need now
- preserves backward compatibility
- creates a clean bridge toward imported identity/profile memory
- prepares backend pluggability without blocking on it

Cons:

- adds new config/runtime types
- summary/profile behavior remains intentionally lightweight in v0.1

### Approach 3: Full Memory Plane Productization

Immediately build configurable backends, retrieval policies, summary stores,
profile stores, migration importers, and cross-session retrieval.

Pros:

- strongest long-term architecture

Cons:

- too large for one safe patch
- high validation burden
- too much product surface before the first stable abstraction exists

## Decision

Adopt Approach 2.

The first memory architecture slice should make behavior pluggable without
pretending multiple storage engines already exist. That means:

- `backend` becomes explicit but still defaults to `sqlite`
- `profile` becomes the user-facing choice
- `mode` is derived internally from `profile`
- provider/chat/channel paths stop calling SQLite window helpers directly

## Proposed User-Facing Memory Profiles

### 1. `window_only`

Behavior:

- inject only the recent sliding window into model context

Use when:

- the operator wants maximum simplicity and predictable token usage

### 2. `window_plus_summary`

Behavior:

- inject a deterministic condensed block for earlier session turns
- inject the recent sliding window after the summary block

Use when:

- the operator wants more continuity without full-history token cost

### 3. `profile_plus_window`

Behavior:

- inject a durable `profile_note` block first when configured
- inject the recent sliding window after it

Use when:

- the operator wants identity/preferences/imported tuning to persist as a
  stable memory lane

## Internal Model

### Memory Backend

Internal field:

- `MemoryBackendKind`

Initial values:

- `sqlite`

### Memory Profile

User-facing field:

- `MemoryProfile`

Initial values:

- `window_only`
- `window_plus_summary`
- `profile_plus_window`

### Memory Mode

Internal field:

- `MemoryMode`

Purpose:

- decouple the user-facing profile from the concrete retrieval behavior
- make future profile-to-mode mapping evolvable

In v0.1, the mapping is 1:1.

## Config Evolution

### Keep

- `memory.sqlite_path`
- `memory.sliding_window`

### Add

- `memory.backend`
- `memory.profile`
- `memory.summary_max_chars`
- `memory.profile_note`

### Backward Compatibility

Old configs that only contain:

- `sqlite_path`
- `sliding_window`

must continue to load and behave like:

- `backend = "sqlite"`
- `profile = "window_only"`

## Runtime Architecture

Introduce a shared memory hydration layer inside `crates/app/src/memory`.

Responsibilities:

- resolve runtime config from app config
- ensure backend-specific reads are hidden behind one API
- build model-ready memory context blocks
- keep chat/provider/channel code free from SQLite-specific retrieval logic

Suggested concepts:

- raw storage operations:
  append turn, load recent window, clear session
- context hydration operations:
  load prompt context, load printable history snapshot

## Summary Strategy For v0.1

The initial summary mode should be deterministic and local.

When the session has more turns than the sliding window:

- older turns are converted into a compact textual summary block
- the summary is trimmed to `summary_max_chars`
- no model call is required

This keeps the implementation fast, cheap, and testable.

## Durable Profile Strategy For v0.1

`profile_note` is the first durable profile lane.

It is intentionally simple:

- plain text
- optional
- injected only for `profile_plus_window`

This is important for migration because imported claw identity, preferences,
and tuning can initially land here before a richer migration bundle exists.

Examples of what may live here later:

- preferred interaction style
- long-lived user preferences
- imported old-claw traits
- nativeized migration notes

## Safety Considerations

- Memory hydration must never silently change runtime permissions.
- Summary/profile blocks must be clearly labeled as derived memory context.
- Imported identity in `profile_note` must not override safety invariants in the
  LoongClaw base prompt.
- High-risk actions still require policy/runtime confirmation independent of
  memory profile.

## Performance Considerations

- Default behavior remains cheap:
  `window_only` with SQLite and fixed `sliding_window`.
- Summary generation is deterministic string processing, not model inference.
- Memory config is converted into typed runtime config once and reused.
- Upper bounds such as `summary_max_chars` keep prompt growth predictable.

## Migration Relevance

This architecture is the first real bridge from "prompt-only inheritance" to
"identity + behavior inheritance":

- prompt pack handles native LoongClaw identity
- memory profile handles how continuity is injected
- `profile_note` becomes the first landing zone for imported claw identity and
  durable preferences

That means migration no longer has to force everything into one giant system
prompt.

## Acceptance Criteria

- Config supports explicit memory backend/profile metadata without breaking old
  TOML files.
- Provider/chat/channel paths hydrate memory through a shared memory layer
  instead of direct SQLite window reads.
- `window_plus_summary` produces a deterministic older-context summary block.
- `profile_plus_window` injects `profile_note` when present.
- Onboarding supports `--memory-profile`.
- Existing SQLite workflows continue to function.
