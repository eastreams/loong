# Memory Retrieval

## User Story

As a LoongClaw operator, I want query-aware memory retrieval with explicit
scope and provenance so that useful older context can be recalled without
turning durable memory into an opaque or identity-overriding prompt side
channel.

## Acceptance Criteria

- [x] LoongClaw exposes retrieval that can reason over an explicit query rather
      than only implicit summary hydration.
- [x] Retrieval can be scoped across the runtime's existing memory scope model
      instead of being permanently fixed to session-local summary only.
- [x] Retrieved artifacts surface provenance that is meaningful to operators,
      including where the result came from and why it was injected.
- [x] Workspace-document retrieval honors explicit record-status metadata so
      inactive records are filtered before ranking or operator inspection.
- [x] The shipped surface includes a local text-search path before any
      embedding-dependent retrieval becomes required.
- [x] Retrieved memory remains advisory and does not override runtime self,
      resolved runtime identity, or other continuity lanes.
- [x] Product docs clearly distinguish the current scoped retrieval surface from
      later embedding-based or hybrid search enhancements.
- [ ] Derived-memory ranking should grow beyond the current metadata-backed FTS
      baseline without weakening provenance or identity boundaries.

## Current Baseline

The current runtime already ships:

- typed `MemoryScope`
- typed canonical memory kinds
- staged memory vocabulary:
  `Derive`, `Retrieve`, `Rank`, `AfterTurn`, `Compact`
- explicit retrieval-request modeling
- runtime-self continuity boundaries
- operator-facing `memory_search`
- operator-facing `memory_get`
- workspace durable-memory filtering that respects record status such as active
  vs tombstoned material

The remaining gaps are not basic retrieval existence. The remaining gaps are:

- trust scoring and TTL or hash-backed durability hints
- richer derived-memory ranking and synthesis
- workflow-aware retrieval overlays that stay advisory
- optional later embedding or hybrid search without changing the authority
  model

## Out of Scope

- mandatory embeddings
- vector-only retrieval
- external memory vendors as prompt authority
- implicit identity promotion from retrieved material
- Web UI memory dashboards
