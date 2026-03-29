# Internal Integration SDK Contract

## Purpose

This document defines the maintainer-facing SDK contract for repository-owned
surfaces inside LoongClaw.

It answers one question:

- how should a maintainer add or evolve a concrete runtime surface without
  scattering identity, validation, and projection logic across the repository?

## Core Thesis

Every mature integration family should have one primary descriptor or registry
seam that feeds its projections.

The maintainer flow should be:

1. declare the surface in the family-owned seam
2. implement the runtime adapter or executor
3. attach validation and support facts beside that seam
4. let config, doctor, catalog, status, and docs consume shared projections
5. add family-specific conformance tests

## Families

### Channels

Channels are the clearest current internal SDK family.

They already have descriptor and registry seams that drive:

- catalog metadata
- config validation
- doctor projections
- runtime-backed vs config-backed status

### Tools

Tools already use a catalog-driven internal seam.

That family should keep using catalog metadata as the source of truth for:

- visibility
- capability action class
- governance profile
- runtime availability

### Memory systems

Memory systems already use a registry-and-metadata pattern.

That structure should be preserved rather than flattened back into ad hoc
backend branching.

### Providers

Providers already have runtime contracts, validation helpers, and transport
abstractions, but their maintainer-facing seam is less obvious than channels.

That makes providers the clearest Phase 1 convergence target.

## Required Properties

Every mature family should expose:

- canonical identity
- descriptor or registry ownership
- projections into operator surfaces
- explicit policy or governance facts when runtime decisions depend on them
- conformance tests that prove descriptor-to-projection alignment

## Product-Mode Compatibility

Internal descriptor layers should increasingly be able to answer support facts
such as:

- whether the surface supports kernel-bound mutation
- whether it supports approval round-trips
- whether it may participate in bounded autonomous acquisition

These must be declared facts, not local prompt heuristics.

## Stability

The internal integration SDK is an internal engineering contract.

It should optimize for:

- coherence
- low drift
- repeatable reviews
- projection consistency

It is not a public compatibility promise.
