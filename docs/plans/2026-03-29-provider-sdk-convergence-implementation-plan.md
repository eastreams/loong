# Provider SDK Convergence Implementation Plan

Date: 2026-03-29
Status: Proposed

## Goal

Turn the provider family into a clearer internal SDK surface so maintainers can
add or evolve a provider without rediscovering scattered seams across config,
runtime contracts, validation, and tests.

This plan is the provider-specific execution slice of Phase 1 in
`docs/plans/2026-03-28-sdk-strategy-implementation-roadmap.md`.

## Why This Slice Exists

LoongClaw has already done meaningful provider-runtime decomposition:

- request-session assembly
- payload shaping
- request dispatch
- failover orchestration
- catalog query flow
- validation runtime
- capability profile overrides

That work improved runtime clarity, but not yet the maintainer authoring seam.

## Current Architecture Evidence

`crates/app/src/config/provider.rs` already owns the strongest static provider
facts:

- canonical id
- aliases
- protocol family
- feature family
- auth defaults
- default headers

`crates/app/src/provider/contracts.rs` already owns the strongest request-time
behavior contract:

- transport mode
- payload adaptation
- validation rules
- capability defaults
- error classification

The missing piece is explicit convergence between those seams and the
projection layer.

## Core Decision

LoongClaw should not introduce a second parallel provider registry.

The right move is:

1. treat `ProviderProfile` as the static provider descriptor seam
2. treat `ProviderRuntimeContract` as the runtime behavior seam
3. make validation, setup guidance, and conformance tests consume those seams
   explicitly

## Deliverables

- one documented maintainer-facing provider family model
- clearer ownership split between `ProviderProfile` and
  `ProviderRuntimeContract`
- reduced local recomputation in validation and setup guidance
- provider-family conformance tests for descriptor uniqueness and
  descriptor-to-runtime alignment
- contributor docs that point maintainers to the correct provider family seams

## Acceptance Criteria

This slice is successful when:

- a maintainer can identify one static descriptor seam for provider identity
  and defaults
- runtime behavior changes can be explained as contract derivation rather than
  local branching
- feature-gate, validation, and auth-guidance projections are traceable to the
  provider descriptor seam
- adding a new provider no longer depends on hidden edits by folklore
