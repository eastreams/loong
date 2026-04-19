# Loong SDK Strategy

## Purpose

This document defines the current SDK strategy for Loong.

Loong should not frame its SDK as a generic plugin story.
It should frame its SDK as a product-governed capability system that serves:

- internal repository-owned integration work
- external community authoring
- governed promotion from runtime evidence to durable assets

Companion documents:

- `docs/design-docs/internal-integration-sdk-contract.md`
- `docs/design-docs/external-authoring-contract.md`
- `docs/design-docs/capability-promotion-contract.md`
- `docs/design-docs/sdk-stability-policy.md`
- `docs/design-docs/sdk-validator-contract.md`
- `eastreams/knowledge-base/loong/implementation-plans`

## Current Direction

Loong is no longer best described as a `discovery-first` product.

The current direction is:

- `discovery-first` as a lower-layer tool-selection substrate
- `product mode` as the product-facing capability-acquisition surface
- autonomy-policy as the internal runtime control kernel
- governed promotion as the path from runtime evidence to durable capability
  assets

That distinction matters because the SDK must serve more than tool lookup.

## Core Thesis

Loong's SDK should be defined by the contracts that let capability:

1. be authored
2. be integrated
3. be acquired
4. be governed
5. be validated
6. be promoted
7. be reused

The center of gravity is not "make extension code easier to write."
The center of gravity is "make new capability enter the system safely,
inspectably, and durably."

## Design Goals

1. Keep `product mode` as the operator-facing vocabulary for capability
   acquisition.
2. Keep autonomy-policy and kernel binding as hard runtime boundaries.
3. Make internal integration work more repeatable for maintainers.
4. Give external authors stable artifact contracts without freezing internal
   crate structure.
5. Make install, inspect, doctor, catalog, and audit consume shared metadata.
6. Converge runtime-derived promotion targets with the same lower-layer asset
   taxonomy used by manual authoring.

## Non-Goals

This strategy does not aim to:

- expose the whole internal Rust module layout as a public SDK
- replace `product mode` with `discovery-first`
- make third-party native in-process plugins the default extension path
- blur runtime policy with future learning or self-modification
- auto-promote runtime behavior into live capability without review

## Four-Layer SDK Model

### 1. Product capability surface

This is the operator-facing layer.

It owns:

- `product mode`
- high-level acquisition posture
- blocked-reason semantics
- operator-visible explanation vocabulary

### 2. Internal integration SDK

This is the maintainer-facing layer.

It owns the repository's descriptor, registry, and projection seams for:

- providers
- tools
- channels
- memory systems
- future workflow or pack families where the same pattern applies

### 3. External authoring contract

This is the public author-facing layer.

It should stabilize:

- package identity
- package layout
- setup metadata
- ownership intent
- validator meaning
- supported runtime lanes

### 4. Governed promotion plane

This is the codification layer between runtime evidence and durable assets.

It should treat promotion as governed artifact generation, not live
self-modification.

## Public Capability Families

The clearest current public capability family is managed skills.

The next public families should converge around:

- manifest-first governed plugin packages
- workflow or flow assets
- promotion artifacts that resolve into bounded lower-layer targets

## Recommended Implementation Order

1. converge internal maintainer seams
2. stabilize external artifact contracts
3. stabilize validator meaning
4. align promotion targets with manual authoring taxonomy
5. only then widen helper APIs or executor automation

## Practical Implication

When Loong says "SDK", it should not mean one helper crate.
It should mean one coherent capability lifecycle across:

- product vocabulary
- maintainer integration seams
- external artifact contracts
- governed promotion
