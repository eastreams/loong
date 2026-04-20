# Capability Promotion Contract

## Purpose

This document defines how validated runtime behavior should become a durable
capability asset in Loong.

Promotion should be treated as governed codification, not live
self-modification.

## Core Thesis

The correct sequence is:

1. run behavior
2. capture evidence
3. derive candidate
4. assess readiness
5. derive a dry-run promotion plan
6. review
7. codify into a bounded lower-layer asset
8. reintroduce that asset through normal governed runtime paths

## Promotion Ladder

### Runtime experiment

Evidence capture only.

### Capability candidate

One explicit codification proposal.

### Capability family

Aggregation and readiness over compatible candidates.

### Promotion plan

Dry-run mapping from one family to one bounded lower-layer target.

### Promotion execution

Future mutation layer, intentionally after the lower layers are explicit.

## Canonical Targets

Current bounded targets are:

- `managed_skill`
- `programmatic_flow`
- `profile_note_addendum`

These are not arbitrary.
They are the lower-layer asset kinds that promotion should converge on instead
of inventing a parallel ecosystem.

## Boundedness Rules

Promotion artifacts should stay explicit about:

- target type
- summary
- bounded scope
- required capabilities
- provenance

An artifact may be valid without being ready.
That distinction is crucial for keeping promotion governed.
