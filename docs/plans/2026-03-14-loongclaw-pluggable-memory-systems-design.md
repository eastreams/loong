# LoongClaw Pluggable Memory Systems Design

Date: 2026-03-14
Status: Approved for phased implementation planning

## Summary

The 2026-03-11 memory slice gave LoongClaw a profile-first memory surface
(`window_only`, `window_plus_summary`, `profile_plus_window`) while keeping
SQLite as the only concrete backend. That was the right first step, but it is
not enough for the next product requirement: user-selectable memory systems.

The system now needs to prepare for integrations that look more like:

- a lightweight memory SDK or managed memory layer
- a local-first cognitive retrieval engine
- a full memory framework or external service

These are not interchangeable storage backends. They sit at different layers of
the memory stack and expose different operating models. The design in this
document keeps LoongClaw in control of canonical history, policy, and context
projection while making derivation and retrieval pluggable.

The recommended direction is:

- keep a LoongClaw-owned canonical fact layer
- treat external memory systems as derivation and retrieval adapters
- keep `ConversationContextEngine` as the prompt/context projection authority
- let ACP and provider turns share the same canonical memory authority
- introduce a typed memory-system selection surface that can remain stable while
  adapters evolve

## Why This Slice Exists

The current memory architecture solved one problem well: users can choose memory
behavior without thinking in storage-engine terms. It does not yet solve the
next problem: allowing operators to choose a memory system style without
fragmenting the runtime into separate memory authorities.

Today, `alpha-test` already has the right raw ingredients:

- a stable kernel memory plane
- app-side memory helpers that centralize prompt hydration
- a pluggable `ConversationContextEngine`
- an ACP control plane intentionally separated from provider turns

That means LoongClaw does not need a rewrite. It needs a stronger architectural
boundary between:

- canonical raw history
- derived memories
- memory retrieval
- final prompt/context projection

## Product Goals

- Support user-selectable memory systems without making users think in raw
  backend terms.
- Preserve the local-first default and the current baseline interaction
  experience.
- Keep one canonical raw-history authority owned by LoongClaw.
- Support both provider-routed and ACP-routed conversation output.
- Allow external memory systems to vary in derivation and retrieval behavior
  without rewriting the conversation runtime.
- Make failure modes explicit and safe: external memory should enrich context,
  not become a hidden single point of failure.

## Non-Goals

- No multi-provider federation in the first slice.
- No mandatory external memory service dependency.
- No rewrite of the current ACP/provider boundary.
- No attempt to force all external memory systems into the same storage shape.
- No handoff of final prompt shaping to an external memory product.
- No requirement that the first external adapter be networked or hosted.

## Current State

The current `alpha-test` branch already draws an important boundary:

- `ConversationContextEngine` is the context-lifecycle and context-projection
  seam, not the storage authority.
- The built-in `DefaultContextEngine` is intentionally light. It reads a memory
  window and assembles prompt messages.
- The `memory` module still behaves like a narrow canonical store and prompt
  hydration layer. Its current public behavior is append-turn, recent-window,
  clear-session, and deterministic prompt hydration.
- The provider path persists user and assistant turns and participates in the
  context-engine lifecycle.
- The ACP path persists raw turns and structured ACP runtime events, but it does
  not reuse the provider-side context-engine lifecycle hooks.

This separation is good. The next step should preserve it.

## External System Taxonomy

The design must account for the fact that "memory system" can mean different
things:

### 1. Memory Layer / SDK

Example shape: Mem0-like systems.

Characteristics:

- derive personalized memories from conversations
- expose APIs like add, search, and user/session scoped retrieval
- may be self-hosted or managed

What LoongClaw should treat them as:

- a derivation and retrieval adapter

### 2. Local Cognitive Retrieval Engine

Example shape: Lucid-like systems.

Characteristics:

- local-first storage and retrieval
- low-latency coding-assistant oriented recall
- stronger opinions about ranking, compression, episodic traces, and
  association

What LoongClaw should treat them as:

- a local derivation and retrieval adapter

### 3. Full Memory Framework / Service

Example shape: Cortex-like systems.

Characteristics:

- its own hierarchy, indexing model, APIs, observability, or MCP surface
- may manage session/user/agent/resource memory as a full subsystem

What LoongClaw should treat them as:

- an external memory subsystem adapter that still sits below LoongClaw's final
  context projection

## Approaches Considered

### Approach 1: Treat Each Memory System As A Backend

Add a single selector like:

```toml
[memory]
backend = "sqlite|lucid|mem0|cortex"
```

Pros:

- simple to describe initially
- low short-term design overhead

Cons:

- conflates raw storage, derivation, retrieval, and projection
- forces unrelated systems into a fake common denominator
- makes future ACP/provider convergence harder
- risks turning external vendors into the only memory authority

### Approach 2: One Monolithic `MemoryProvider` Abstraction

Expose a single app-layer trait that stores turns, derives memory, retrieves
results, and shapes prompt input.

Pros:

- cleaner than backend-switching
- one registry and one selection surface

Cons:

- still mixes authority, derivation, retrieval, and prompt shaping
- hides important failure modes
- makes it too easy for one adapter to take over prompt assembly
- eventually becomes a "god trait" that blocks clean evolution

### Approach 3: Layered Memory Stack With LoongClaw Canonical Authority

Keep LoongClaw as the canonical raw-history authority. Introduce explicit layers
for derivation and retrieval. Keep `ConversationContextEngine` as the final
projection layer.

Pros:

- preserves LoongClaw control over truth, policy, and context assembly
- supports local and hosted systems equally well
- works for both provider and ACP-produced history
- allows product-level memory-system choice without rewriting the runtime
- gives a clean migration path from today's SQLite-first implementation

Cons:

- slightly more architecture to introduce up front
- requires discipline to keep layers separate

## Decision

Adopt Approach 3.

LoongClaw should not choose one memory product as its primary authority. It
should own canonical facts and let memory systems compete and evolve at the
derivation and retrieval layers.

## Design Principles

### 1. Canonical First

LoongClaw keeps the canonical raw conversation and runtime-event stream. That
stream is replayable, inspectable, and suitable for re-indexing.

### 2. Profile First, System Second, Backend Last

The user-facing choice remains about memory behavior and system style, not about
raw storage engines. Storage remains an implementation detail as long as
possible.

### 3. Projection Stays In LoongClaw

External memory systems may return retrieved records or hierarchical artifacts,
but they do not directly author LoongClaw's final prompt.

### 4. ACP And Provider Share One Fact Layer

ACP should not grow into a separate long-term memory authority. ACP-generated
turns and events should still feed the same canonical layer as provider turns.

### 5. Degrade Gracefully

External memory-system failure must fail open. Baseline interaction continues
with built-in recent-context behavior.

### 6. Typed Artifacts Over Opaque Prompt Blobs

Memory derivation should produce typed records, not only preformatted text.
Typed artifacts keep retrieval, ranking, auditing, and prompt projection under
control.

## Proposed Conceptual Architecture

### Layer 1: Canonical Memory Store

Responsibility:

- persist raw conversation turns and runtime events
- provide deterministic recent-window reads
- provide replayable session history for reindexing and migration

Initial authority:

- the existing SQLite-backed LoongClaw memory plane

Suggested conceptual operations:

- `append_record`
- `window`
- `session_log`
- `clear_session`

Suggested conceptual record:

```rust
struct CanonicalMemoryRecord {
    record_id: String,
    session_id: String,
    scope: MemoryScope,
    kind: CanonicalMemoryKind,
    role: Option<String>,
    content: String,
    ts: i64,
    metadata: serde_json::Value,
}
```

Initial `CanonicalMemoryKind` should be broad enough for:

- user turn
- assistant turn
- tool result
- imported profile
- ACP runtime event
- ACP final event

### Layer 2: Memory Derivation

Responsibility:

- transform canonical records into durable higher-order memory artifacts
- run synchronously in minimal mode or asynchronously in richer modes

Derived artifact types may include:

- summary
- profile
- fact
- entity
- episode
- procedure
- hierarchical abstract or overview records

Suggested conceptual output:

```rust
struct DerivedMemoryRecord {
    record_id: String,
    source_record_ids: Vec<String>,
    scope: MemoryScope,
    kind: DerivedMemoryKind,
    content: String,
    metadata: serde_json::Value,
}
```

Examples:

- built-in mode: deterministic summary and profile-note projection
- Lucid-like mode: local episodic and procedural traces
- Mem0-like mode: personalized fact extraction and search index updates
- Cortex-like mode: hierarchical L0/L1/L2 style artifacts

### Layer 3: Memory Retrieval

Responsibility:

- answer context queries using derived memory under explicit policy and budget
- return typed candidate records plus diagnostics

Suggested conceptual request:

```rust
struct MemoryRetrievalRequest {
    session_id: String,
    query: Option<String>,
    scopes: Vec<MemoryScope>,
    budget_items: usize,
    allowed_kinds: Vec<DerivedMemoryKind>,
}
```

Important rule:

- recent conversation window is not replaced by semantic retrieval
- retrieval augments context; it does not become the only context source

### Layer 4: Memory Orchestrator

LoongClaw needs one app-layer orchestrator that composes canonical store,
derivation, and retrieval behind a stable runtime API.

Responsibilities:

- persist canonical records
- fan out to derivation
- load recent window
- query derived memory
- produce a typed hydrated memory snapshot for context projection

Suggested conceptual output:

```rust
struct HydratedMemoryContext {
    recent_window: Vec<CanonicalMemoryRecord>,
    retrieved: Vec<DerivedMemoryRecord>,
    diagnostics: MemoryDiagnostics,
}
```

This is the natural evolution of today's `load_prompt_context(...)`.

### Layer 5: Context Projection

`ConversationContextEngine` remains responsible for:

- prompt ordering
- system-prompt addition
- token budgeting
- compaction policy
- subagent lifecycle hooks

The context engine should consume a hydrated memory snapshot from the memory
orchestrator rather than owning storage or derivation itself.

## Proposed Runtime Surface

The conceptual design should map to an initial runtime surface that is real but
not over-engineered.

### Stable User-Facing Surface

Keep existing memory fields:

- `memory.backend`
- `memory.profile`
- `memory.sqlite_path`
- `memory.sliding_window`
- `memory.summary_max_chars`
- `memory.profile_note`

Add:

- `memory.system`
- `memory.fail_open`
- `memory.ingest_mode`

Suggested meaning:

- `backend`: canonical raw-store backend
- `profile`: user-visible behavior contract
- `system`: selected memory-system family
- `ingest_mode`: whether derivation prefers synchronous minimal work or
  asynchronous background work

### Advanced Adapter Config

Reserve nested adapter config for later:

```toml
[memory]
backend = "sqlite"
profile = "window_only"
system = "builtin"
fail_open = true
ingest_mode = "sync_minimal"

[memory.systems.lucid]
enabled = true

[memory.systems.mem0]
enabled = true
api_key_env = "MEM0_API_KEY"

[memory.systems.cortex]
enabled = true
endpoint = "http://127.0.0.1:8085"
```

The nested tables are intentionally adapter-specific. LoongClaw should not
pretend these systems have identical configuration models.

## Proposed Selection Model

This slice should separate three distinct selectors:

### 1. Canonical Backend

What stores raw canonical facts?

Initial value:

- `sqlite`

### 2. Memory System

Which derivation and retrieval family is active?

Initial value:

- `builtin`

Future examples:

- `lucid`
- `mem0`
- `cortex`

### 3. Memory Profile

What behavior contract should the user experience?

Current values:

- `window_only`
- `window_plus_summary`
- `profile_plus_window`

Future values can remain independent from adapter brands.

## Proposed Registry Strategy

LoongClaw already has a successful pattern with `context_engine_registry`.
Memory should follow the same spirit, but not necessarily the same exact
granularity on day one.

### Recommended Phase-1 Code Shape

Introduce one selected memory-system facade:

- `MemorySystem`
- `MemorySystemMetadata`
- `resolve_memory_system(...)`
- `list_memory_systems(...)`

Internally, the built-in memory system can still compose:

- canonical store
- derivation
- retrieval

This keeps the first implementation slice simple.

### Future Split When Needed

Only split into separate public registries for:

- `CanonicalMemoryStore`
- `MemoryDeriver`
- `MemoryRetriever`

after LoongClaw proves it truly needs mix-and-match composition rather than one
selected stack per runtime.

## Memory Scope Model

The system should standardize scope names now so future adapters have a stable
mapping target.

Suggested scopes:

- `session`
- `user`
- `agent`
- `workspace`

These are enough to cover the near-term product direction without overcommitting
to more exotic shapes.

## Provider And ACP Data Flow

### Provider Path

1. Persist canonical user turn.
2. Perform minimal derivation work if configured.
3. Build context through `ConversationContextEngine`, which reads hydrated
   memory from the memory orchestrator.
4. Request provider turn.
5. Persist canonical assistant turn.
6. Trigger post-turn derivation and retrieval maintenance.

### ACP Path

1. Route through ACP manager as today.
2. Persist canonical raw user turn or runtime event before or during ACP
   execution.
3. Persist assistant final result and structured ACP runtime events.
4. Feed those canonical records into derivation.
5. Keep ACP separate from provider-only context-engine lifecycle hooks unless a
   new explicit ACP-aware seam is added later.

This preserves the existing ACP architecture while preventing memory split-brain.

## Failure Policy

### Canonical Store Failure

Preserve current runtime semantics. Do not let new external memory features make
canonical persistence less reliable.

### Derivation Failure

Fail open:

- log diagnostics
- keep recent-window behavior
- avoid blocking the main conversation turn unless the operator explicitly
  chooses strict mode in the future

### Retrieval Failure

Fail open:

- skip external retrieval output
- continue with built-in context projection

## Migration Strategy

### Phase 0: Document The Boundary

- no behavior change
- no user-visible config break

### Phase 1: Introduce Memory-System Metadata And Selection

- keep built-in system only
- add registry, metadata, config selection, and diagnostics

### Phase 2: Introduce The Memory Orchestrator

- evolve `load_prompt_context(...)` into a richer hydrated-memory API
- preserve current prompt behavior for `system = "builtin"`

### Phase 3: Broaden Canonical Records

- add typed canonical event persistence for ACP and future tool/runtime events
- make replay and re-derivation explicit

### Phase 4: Add One Experimental External Adapter

Recommendation:

- start with a local-first adapter before a hosted one

Reason:

- lowest operational risk
- best fit for baseline UX preservation
- easiest way to validate the abstractions without introducing service
  dependency complexity

### Phase 5: Reassess Public Registry Granularity

- keep one memory-system facade if it remains sufficient
- split component registries only if adapter reality demands it

## Testing Strategy

The architecture is only useful if it stays stable under current behavior.

Required validation:

- backward-compatibility tests for old memory config
- selection tests for built-in memory system metadata and resolution
- prompt-behavior regression tests for built-in recent-window flow
- provider-path characterization tests
- ACP-path characterization tests proving shared canonical persistence does not
  imply shared provider lifecycle hooks
- fail-open tests for derivation and retrieval failure

## Consequences

### Positive

- supports future user-selectable memory systems without product lock-in
- preserves LoongClaw control over truth and prompt assembly
- lets ACP and provider share one fact layer without forcing one turn pipeline
- makes future migration and reindexing possible

### Negative

- adds another layer of abstraction before the second real adapter lands
- requires discipline to avoid leaking external-system prompt formatting into
  core runtime logic

## Final Recommendation

LoongClaw should evolve toward a pluggable memory-stack architecture, but it
should do so from a strong center:

- canonical facts belong to LoongClaw
- derivation and retrieval are pluggable
- context projection remains a LoongClaw responsibility
- user choice is expressed as profile plus selected memory system, not as raw
  backend wiring

That gives the project a clean foundation for Lucid-like, Mem0-like, and
Cortex-like integrations without surrendering architecture control to any one of
them.
