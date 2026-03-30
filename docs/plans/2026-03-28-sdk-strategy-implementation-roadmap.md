# SDK Strategy Implementation Roadmap

Date: 2026-03-28
Status: Proposed

## Goal

Turn the SDK strategy from architecture framing into a phased execution
program.

## Strategic Frame

Implementation should follow LoongClaw's current direction:

- `product mode` stays the operator-facing capability vocabulary
- autonomy-policy becomes the internal runtime control kernel
- internal SDK seams reduce maintainer wiring cost
- external authoring contracts define public package shape
- promotion contracts connect runtime evidence to reusable assets

## Success Conditions

This roadmap is successful when:

- new repository-owned surfaces can be added through repeatable family seams
- public authoring no longer depends on internal crate layout
- install, doctor, catalog, runtime visibility, and audit agree on shared
  metadata
- promotion targets align with the same lower-layer asset taxonomy used by
  manual authoring

## Phase 1: Internal SDK Convergence

Focus:

- strengthen provider integration into a clearer maintainer-facing contract
- keep channels on descriptor-driven seams
- keep tools and memory systems projection-driven
- add family-specific conformance tests

Deliverables:

- documented maintainer flow per integration family
- provider SDK convergence plan for the least unified core family
- clearer provider descriptor or contract seam
- contributor guidance aligned with real family seams

## Phase 2: Product-Mode And Autonomy Support Facts

Focus:

- make more surfaces declare support facts for approval, mutation, and bounded
  autonomous acquisition

## Phase 3: Public Authoring Contract Baseline

Focus:

- consolidate managed skills and manifest-first packages into a clearer public
  contract
- define validator behavior and compatibility expectations

## Phase 4: Supply Chain And Governance Alignment

Focus:

- trust tiers
- provenance
- ownership conflict handling
- stronger install, doctor, and audit alignment

## Phase 5: Promotion Contract Convergence

Focus:

- align runtime-derived promotion targets with manually authored asset
  taxonomy

## Phase 6: Bounded Promotion Execution

Focus:

- design bounded mutation only after promotion inputs and dry-run plans are
  explicit enough

## Phase 7: Learning And Ranking Layers

Focus:

- keep future optimization above stable contracts rather than inside hard
  runtime permissions
