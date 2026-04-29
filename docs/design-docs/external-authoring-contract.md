# External Authoring Contract

## Purpose

This document defines the public-facing authoring contract for Loong
capability artifacts.

Its purpose is to let community authors build capability packages without
depending on internal crate layout.

## Core Thesis

The public SDK should be contract-first, package-first, and artifact-first.

Loong should stabilize:

- package identity
- package layout
- setup semantics
- ownership semantics
- validator meaning
- install, inspect, and audit behavior

before trying to stabilize internal helper APIs.

## Public Capability Families

### Managed skills

Managed skills are the clearest current public capability family.

They are:

- installable
- inspectable
- operator-visible
- compatible with bounded acquisition flows
- natural promotion targets

### Governed plugin packages

These packages should remain manifest-first and lane-aware.

They should declare:

- identity
- capability intent
- setup metadata
- ownership intent
- supported runtime lane

without implying trusted in-process execution.

### Native Loong extensions

Native extensions are the lowest-friction authoring lane on top of governed
plugin packages.

The intended ergonomic shape is:

- scaffold a package root,
- write a small local runtime file,
- inspect declarations before execution,
- smoke-test the runtime file through the governed bridge,
- then promote the package through normal inventory / doctor / audit loops.

For the current public authoring lane, Loong should stabilize:

- `loong plugins init` for manifest-first scaffolding
- runnable local `process_stdio` entrypoints for the supported public templates
- `loong plugins invoke-extension` as a bounded smoke surface
- inventory / doctor / operator surfaces that show extension declarations before execution
- explicit extension family and trust-lane identity for the current governed native runtime lane

without implying ungoverned in-process host loading.

The currently supported public runnable templates are:

- Python
- JavaScript
- Go
- Rust

### Workflow and flow assets

These are strategically important, especially because promotion already points
at `programmatic_flow` as a target family.

They are still less concrete than managed skills today.

## Public Principles

Every public artifact family should follow the same rules:

- explicit capabilities
- explicit metadata
- explicit setup surface
- explicit ownership and intent
- controlled runtime lanes
- installability and inspectability

Runtime inspectability should expose declared extension metadata before any
runtime invocation. At minimum, operator-visible snapshot and audit surfaces
should be able to show:

- the declared extension contract
- declared facets
- declared host-facing methods
- declared events
- declared host actions

without requiring the host to execute extension code first.

## What Is Not Promised

The public contract should not promise:

- internal `crates/app` helper APIs
- internal registry organization
- executor layout
- automatic self-evolution behavior

Those remain internal or experimental until proven durable.
