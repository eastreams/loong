# Memory Retrieval

## User Story

As a LoongClaw operator, I want query-aware memory retrieval with explicit
scope and provenance so that useful older context can be recalled without
turning durable memory into an opaque or identity-overriding prompt side
channel.

## Acceptance Criteria

- [ ] LoongClaw exposes retrieval that can reason over an explicit query rather
      than only implicit summary hydration.
- [ ] Retrieval can be scoped across the runtime's existing memory scope model
      instead of being permanently fixed to session-local summary only.
- [ ] Retrieved artifacts surface provenance that is meaningful to operators,
      including where the result came from and why it was injected.
- [ ] Workspace-document retrieval honors explicit record-status metadata so
      inactive records are filtered before ranking or operator inspection.
- [ ] The first slice includes a local text-search path before any
      embedding-dependent retrieval becomes required.
- [ ] Retrieved memory remains advisory and does not override runtime self,
      resolved runtime identity, or other continuity lanes.
- [ ] Product docs clearly distinguish first-slice scoped retrieval from later
      embedding-based or hybrid search enhancements.

## Current Baseline

The current runtime already ships:

- typed `MemoryScope`
- typed canonical memory kinds
- staged memory vocabulary:
  `Derive`, `Retrieve`, `Rank`, `AfterTurn`, `Compact`
- explicit retrieval-request modeling
- runtime-self continuity boundaries

The built-in retrieval path is still intentionally narrow:

- no explicit query
- session scope only
- summary kind only

## Out of Scope

- mandatory embeddings
- vector-only retrieval
- external memory vendors as prompt authority
- implicit identity promotion from retrieved material
- Web UI memory dashboards
