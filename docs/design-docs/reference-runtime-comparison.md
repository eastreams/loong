# Reference Runtime Comparison and Memory Convergence Roadmap

## Scope and Baseline

- LoongClaw baseline: `loongclaw-ai/loongclaw` `dev@c8e38923` (inspected on
  2026-03-20).
- External references inspected on 2026-03-20:
  - Hermes Agent official memory and skills docs
  - OpenClaw official memory, memory-config, and plugin docs
  - DeepAgents official long-term memory and customization docs

This document answers one practical question:

How should LoongClaw evolve its memory system so it becomes competitive on
continuity and recall without giving up its stronger architecture boundaries?

The conclusion is intentionally narrow:

- LoongClaw already has a better memory authority boundary than most reference
  systems.
- LoongClaw does not yet have a complete memory product.
- The next slice should add bounded hot memory plus episodic recall on top of
  LoongClaw-owned canonical history.

## Executive Summary

Hermes Agent, OpenClaw, and DeepAgents each solve a different memory problem
well:

- Hermes Agent is the strongest reference for cache-aware layered memory.
- OpenClaw is the strongest reference for operator-visible workspace memory and
  retrieval.
- DeepAgents is a useful control case for durable execution and file-backed
  state, but it is not yet a strong reference for layered agent memory.

LoongClaw today is strongest where the others are usually weakest:

- one canonical authority for raw history
- typed canonical records beyond plain chat turns
- explicit separation between storage, retrieval, and final context projection
- fail-open design for future external memory adapters

LoongClaw today is weakest where the others are already operator-valuable:

- no bounded durable hot memory available on every turn
- no first-class episodic recall tool comparable to `session_search` or
  `memory_search`
- no mature procedural-memory story that turns repeated successful workflows
  into reusable memory artifacts
- no pre-compaction durable-memory flush

The recommendation is not to copy Hermes or OpenClaw literally.

The recommendation is to keep LoongClaw's canonical-first architecture and add
the missing product layers in this order:

1. bounded hot memory
2. episodic recall over canonical history
3. typed derived-memory artifacts
4. compaction-time memory flush
5. procedural-memory convergence with skills
6. external derivation and retrieval adapters

## Current LoongClaw Memory Reality

### What exists today

LoongClaw's current `dev` branch has a real memory foundation:

- recent-window prompt hydration from SQLite-backed turns
- deterministic summary checkpoints for `window_plus_summary`
- optional `profile_note` projection for `profile_plus_window`
- typed canonical records for user turns, assistant turns, tool decisions,
  tool outcomes, imported profile entries, and ACP events
- a `MemorySystem` registry and orchestrator seam
- a `ConversationContextEngine` that remains the final prompt projection
  authority

This is materially better than a transcript-only memory design.

It means LoongClaw can already preserve a replayable canonical event stream and
can later derive richer memory artifacts without surrendering prompt authorship
to an external system.

### What does not exist today

LoongClaw still does not ship a mature long-term memory product:

- no durable hot memory store comparable to Hermes's `MEMORY.md` / `USER.md`
- no agent-facing `session_search` / `memory_search` style recall tool
- no semantic or hybrid retrieval in the builtin system
- no automatic long-term memory derivation pipeline
- no compaction-time durable-memory extraction stage
- no intentional procedural-memory integration for skills

This is why LoongClaw currently feels like:

- deterministic recent context
- optional deterministic summary
- optional static profile block

That is a useful baseline, but it is not enough to solve issue #356-style
operator problems such as topic switching, shared user knowledge, and durable
cross-session recall.

### Why the current architecture still matters

LoongClaw's current memory gap is a product gap, not an architecture-collapse
gap.

That distinction matters.

The project already made several decisions that should be preserved:

- canonical raw history remains LoongClaw-owned
- external memory systems stay below final prompt projection
- typed records are preferred over opaque prompt blobs
- failure of future derivation or retrieval layers should fail open to recent
  context

These boundaries are worth keeping even if external systems appear to offer
faster short-term feature gains.

## Comparative Snapshot

The qualitative ratings below are this document's synthesis from the linked
reference docs and the repo evidence listed later in this document, not the
reference projects' own labels.

| Dimension | Hermes Agent | OpenClaw | DeepAgents | LoongClaw (`dev`) |
| --- | --- | --- | --- | --- |
| Canonical source of truth | bounded prompt memory + SQLite session archive ([Hermes memory docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#how-it-works), [session search](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#session-search)) | workspace Markdown, plus plugin-managed search indexes ([OpenClaw memory](https://docs.openclaw.ai/concepts/memory#memory-files-markdown), [memory config](https://docs.openclaw.ai/reference/memory-config)) | state/checkpoints + optional persistent file paths ([DeepAgents customization](https://docs.langchain.com/oss/python/deepagents/customization#backends), [long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory#path-routing)) | SQLite canonical history owned by LoongClaw |
| Hot memory | strong ([Hermes memory docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#how-it-works)) | moderate ([OpenClaw memory](https://docs.openclaw.ai/concepts/memory#memory-files-markdown)) | weak ([DeepAgents long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory#how-it-works)) | weak |
| Episodic recall | strong ([Hermes session search](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#session-search)) | strong ([OpenClaw memory tools](https://docs.openclaw.ai/concepts/memory#memory-tools), [memory config](https://docs.openclaw.ai/reference/memory-config)) | weak ([DeepAgents long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory#cross-thread-persistence)) | weak |
| Prompt-cache awareness | strong ([Hermes memory prompt projection](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#how-memory-appears-in-the-system-prompt)) | moderate ([OpenClaw memory](https://docs.openclaw.ai/concepts/memory#memory-files-markdown)) | weak ([DeepAgents long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory#how-it-works)) | weak to moderate |
| Procedural memory | strong ([Hermes skills system](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills#using-skills), [progressive disclosure](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills#progressive-disclosure)) | moderate ([OpenClaw plugin tools](https://docs.openclaw.ai/tools/plugin), [memory tools](https://docs.openclaw.ai/concepts/memory#memory-tools)) | weak ([DeepAgents customization](https://docs.langchain.com/oss/python/deepagents/customization), [long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory)) | weak |
| Architecture boundary cleanliness | good | moderate | moderate | strong |
| External memory adapter posture | optional Honcho layer ([Hermes Honcho integration](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#honcho-integration-cross-session-user-modeling)) | plugin slot and sidecars ([OpenClaw plugin docs](https://docs.openclaw.ai/tools/plugin), [memory config](https://docs.openclaw.ai/reference/memory-config)) | backend routing to stores/filesystems ([DeepAgents customization](https://docs.langchain.com/oss/python/deepagents/customization#backends), [long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory#path-routing)) | explicit derivation/retrieval adapter design |

## Hermes Agent

### What Hermes gets right

Hermes has the clearest layered memory model among the references inspected.

Its official docs show four meaningful layers:

- bounded persistent memory in `MEMORY.md` ([Hermes memory docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#how-it-works))
- bounded user profile memory in `USER.md` ([Hermes memory docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#how-it-works))
- SQLite-backed `session_search` for cross-session recall ([Hermes session search](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory#session-search))
- skills as procedural memory ([Hermes skills system](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills#using-skills))

The most important architectural choice is not the file format.
It is the split between:

- tiny, always-in-context memory
- on-demand, larger recall

Hermes keeps durable prompt memory small on purpose, freezes it at session
start, and pushes broader recall to tools.

That gives Hermes two advantages:

- better prompt stability and cache friendliness
- better discipline about what belongs in prompt memory

### What LoongClaw should copy from Hermes

LoongClaw should copy Hermes's layering principle, not its file layout.

The important lessons are:

- keep hot memory tiny and operator-visible
- do not overload hot memory with transcripts or task diary entries
- use a separate episodic recall path for past conversations
- treat procedural reuse as its own memory layer
- perform durable-memory extraction before lossy context compaction

### What LoongClaw should not copy from Hermes

LoongClaw should not switch its canonical authority to prompt-memory files.

Hermes's file-backed hot memory is appropriate for Hermes because that product
optimizes for a cache-stable prompt prefix and relatively lightweight local
state.

LoongClaw already has a stronger typed canonical layer. It should preserve that
advantage and implement hot memory as typed derived artifacts projected into the
prompt, not as the new storage authority.

## OpenClaw

### What OpenClaw gets right

OpenClaw's current docs show a more capable memory product than the older
"Markdown only" impression suggests.

Today OpenClaw provides:

- `MEMORY.md` plus append-only daily logs under `memory/YYYY-MM-DD.md` ([OpenClaw memory files](https://docs.openclaw.ai/concepts/memory#memory-files-markdown))
- agent-facing `memory_search` and `memory_get` ([OpenClaw memory tools](https://docs.openclaw.ai/concepts/memory#memory-tools))
- automatic pre-compaction memory flush ([OpenClaw pre-compaction flush](https://docs.openclaw.ai/concepts/memory#automatic-memory-flush-pre-compaction-ping))
- hybrid retrieval options including BM25 and vector search ([OpenClaw vector memory search](https://docs.openclaw.ai/concepts/memory#vector-memory-search), [memory config](https://docs.openclaw.ai/reference/memory-config))
- optional session transcript indexing ([memory config](https://docs.openclaw.ai/reference/memory-config))
- install-on-demand long-term memory through `memory-lancedb` ([memory config](https://docs.openclaw.ai/reference/memory-config))

This produces a very operator-legible memory model:

- if it matters, write it to workspace memory
- if you need it later, search the workspace memory

That is easy to explain and easy to inspect.

### What LoongClaw should copy from OpenClaw

LoongClaw should copy three things:

- operator-visible memory surfaces
- on-demand retrieval instead of prompt stuffing
- pre-compaction durable-memory preservation

OpenClaw is especially useful as a reminder that memory is not only an internal
runtime concern. It is also a user and operator experience.

### What LoongClaw should not copy from OpenClaw

LoongClaw should not adopt "workspace notes become the ultimate truth" as its
primary memory authority.

That model works well for OpenClaw's workspace-first product shape, but it would
weaken several LoongClaw goals:

- ACP and provider outputs sharing one canonical fact layer
- typed tool and runtime event persistence
- deterministic auditing and replay
- future re-derivation from raw canonical history

OpenClaw is a strong product reference and a weaker authority model for
LoongClaw's runtime.

## DeepAgents

### What DeepAgents contributes as a reference

DeepAgents is best treated as a control case rather than a direct memory target.

Its official docs emphasize:

- checkpoint-backed thread persistence ([DeepAgents customization](https://docs.langchain.com/oss/python/deepagents/customization), [LangGraph persistence note](https://docs.langchain.com/oss/python/langgraph/persistence))
- transient filesystem state inside the agent runtime ([DeepAgents long-term memory](https://docs.langchain.com/oss/python/deepagents/long-term-memory#1-short-term-transient-filesystem), [DeepAgents customization backends](https://docs.langchain.com/oss/python/deepagents/customization#backends))
- optional long-term persistence by routing specific filesystem paths into a
  durable store ([DeepAgents path routing](https://docs.langchain.com/oss/python/deepagents/long-term-memory#path-routing), [cross-thread persistence](https://docs.langchain.com/oss/python/deepagents/long-term-memory#cross-thread-persistence))
- additional memory and skill files can be injected through the selected backend
  before agent startup ([DeepAgents customization](https://docs.langchain.com/oss/python/deepagents/customization#backends))

This is useful because it demonstrates a clean split between:

- per-thread durable execution
- file-backed transient working state
- opt-in persistent memory paths

### Why DeepAgents is not the main comparison target

DeepAgents is not currently a stronger memory product than Hermes or OpenClaw.

It does not present, at least in the inspected docs, a comparably mature stack
for:

- bounded hot memory
- dedicated episodic recall tools
- procedural-memory learning loops
- compaction-aware memory preservation

Its main value for LoongClaw is as a reminder that durable execution state and
memory are related but not identical concerns.

LoongClaw should preserve that distinction.

## Convergence Decisions

### 1. Keep canonical raw history inside LoongClaw

This remains the correct architecture choice.

Future memory systems should operate as:

- derivation adapters
- retrieval adapters
- optional ranking or summarization helpers

They should not become the prompt authority.

### 2. Add a real hot-memory layer

LoongClaw needs a small bounded memory layer that captures facts which should be
available on every turn.

This should be:

- operator-visible
- typed
- bounded by projection budgets
- separated from transcripts and temporary session progress

The project should not wait for semantic search or external vendors before
adding this layer.

### 3. Add episodic recall before semantic-vendor integration

The highest-leverage next retrieval feature is not vector search.

It is a builtin episodic recall path over LoongClaw's canonical history,
preferably SQLite FTS-backed first.

Why:

- LoongClaw already stores canonical turns locally
- FTS is deterministic, cheap, debuggable, and easy to audit
- this solves "did we talk about X before?" earlier than a more ambitious
  vector stack

### 4. Derive typed memory artifacts instead of injecting opaque blobs

The existing pluggable-memory design is correct here.

Derived memory should become typed artifacts such as:

- profile
- fact
- episode
- procedure
- summary

That gives LoongClaw better control over:

- retrieval ranking
- projection budgets
- auditability
- future migration to richer local or hosted adapters

### 5. Treat compaction as a memory boundary, not only a token boundary

Before any lossy compaction lands, LoongClaw should add a pre-compaction
durable-memory flush.

Otherwise compaction will silently erase exactly the durable facts the future
memory system needs.

### 6. Converge skills with procedural memory after the fact layer exists

Procedural memory is important, but it should land after hot memory and episodic
recall.

The likely shape is:

- successful multi-step workflows generate `procedure` artifacts
- those artifacts can reference installed skills or evolve into them
- retrieval can surface procedure candidates without injecting every skill body
  into the prompt

## Recommended First Slice

The best next slice is not:

- vector search
- memory vendor integration
- topic-specific memory routing
- auto-generated long-term summaries

The best next slice is:

### Layered Memory Continuity Foundation

It should include four concrete outcomes:

1. Builtin durable memory artifacts for bounded hot memory
2. Prompt projection that can include bounded hot memory plus recent context
3. Builtin episodic recall over canonical history
4. Operator-visible diagnostics and policy for these layers

This slice is the smallest change that meaningfully closes the product gap while
preserving LoongClaw's better architecture.

Execution plan:

- [Memory Layered Continuity Foundation Implementation Plan](../plans/2026-03-20-memory-layered-continuity-foundation-implementation-plan.md)

## Follow-On Slices

Once the first slice lands, the recommended order is:

### Slice 2: Automatic derivation and compaction-aware preservation

- derive facts and episodes from canonical history
- add pre-compaction durable-memory flush
- keep fail-open fallback to recent-window behavior

### Slice 3: Procedural-memory convergence

- connect successful workflows to `procedure` artifacts
- integrate with external skills without turning skills into prompt-authority
  blobs

### Slice 4: External retrieval and modeling adapters

- local cognitive retrieval engines
- managed memory SDKs
- richer user-modeling layers

All of them should remain below LoongClaw's final context projection.

The broader memory and knowledge delivery track now lives under epic #421.
That is the right home for file-backed knowledge, hybrid or semantic retrieval,
and per-agent scoping. The foundation slice in this document should land
beneath that epic as a prerequisite, not expand to absorb the whole backlog at
once.

## Why This Direction Fits Issue #356

Issue #356 asks for:

- topic-level independent memory spaces
- shared base user knowledge
- cleaner switching between contexts

The first slice above does not solve the whole topic-isolation problem, but it
builds the right prerequisites:

- shared user facts can live in durable hot memory
- episodic recall can retrieve topic-specific prior sessions
- future scope-aware artifacts can separate session, user, agent, and workspace
  memory more cleanly than today's flat prompt window

That makes the eventual solution architectural rather than ad hoc.

## Final Judgement

Hermes Agent is the best reference for memory layering.
OpenClaw is the best reference for operator-facing memory workflows.
DeepAgents is the best reference for separating durable execution state from
memory concerns.

LoongClaw should not try to "beat" those systems by copying their storage
surfaces.

LoongClaw should beat them by combining:

- stronger canonical authority
- typed derived artifacts
- bounded hot memory
- deterministic episodic recall
- optional external adapters below prompt projection

That is the most defensible path from today's memory foundation to a real memory
product.

## LoongClaw Repo Evidence

The comparison above is grounded in the current repo state, especially:

- [Harness Engineering](harness-engineering.md) for the explicit statement that
  working context and session state exist while long-term memory does not yet.
- [Memory Profiles](../product-specs/memory-profiles.md) for the current
  user-facing memory surface and its out-of-scope boundaries.
- [LoongClaw Pluggable Memory Systems Design](../plans/2026-03-14-loongclaw-pluggable-memory-systems-design.md)
  for the canonical-first, typed-artifact, derivation/retrieval architecture.
- [LoongClaw Pluggable Memory Systems Implementation Plan](../plans/2026-03-14-loongclaw-pluggable-memory-systems-implementation.md)
  for the already-landed memory-system registry and orchestrator foundation.

## References

- Hermes Agent memory docs:
  <https://hermes-agent.nousresearch.com/docs/user-guide/features/memory/>
- Hermes Agent skills docs:
  <https://hermes-agent.nousresearch.com/docs/user-guide/features/skills/>
- OpenClaw memory docs:
  <https://docs.openclaw.ai/concepts/memory>
- OpenClaw memory configuration docs:
  <https://docs.openclaw.ai/reference/memory-config>
- OpenClaw plugin docs:
  <https://docs.openclaw.ai/tools/plugin>
- DeepAgents long-term memory docs:
  <https://docs.langchain.com/oss/python/deepagents/long-term-memory>
- DeepAgents customization docs:
  <https://docs.langchain.com/oss/python/deepagents/customization>
