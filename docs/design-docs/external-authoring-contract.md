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
- setup metadata
- ownership intent
- supported runtime lane

without implying trusted in-process execution.

### Trusted host extensions

Trusted host extensions are the narrow high-trust family for host-facing
runtime seams.

They currently support:

- scaffolded package creation through `loong plugins init`
- declared host hooks
- declared shell-first TUI surfaces
- bounded probe execution through the Loong CLI
- operator-visible runtime and doctor truth

Current shell-first TUI surface declarations include:

- `command_palette`
- `settings_flow`
- `startup_onboarding`

### Workflow and flow assets

These are strategically important, especially because promotion already points
at `programmatic_flow` as a target family.

They are still less concrete than managed skills today.

## Public Principles

Every public artifact family should follow the same rules:

- explicit metadata
- explicit setup surface
- explicit ownership and intent
- controlled runtime lanes
- installability and inspectability

Trusted host extensions add one more rule:

- high-trust execution must stay explicit, bounded, and inspectable

## Current Operator Surfaces

Today, an external author can rely on these operator-visible surfaces:

- `loong plugins init --host-hook ...`
- `loong plugins init --tui-surface ...`
- `loong plugins inventory`
- `loong plugins doctor`
- `loong plugins invoke-host-hook`
- `loong plugins invoke-tui-surface`
- shell-first TUI inspection with `/extensions`
- shell-first bounded TUI probe routing with `/extensions run <plugin-id> <surface>`

The live shell-first TUI currently routes these declared surfaces:

- `command_palette`
- `settings_flow`
- `startup_onboarding`

That routing is additive. It does not yet imply a general in-process TUI
executor contract.

## Current Trusted Host Scaffold Boundary

Today, the scaffolded trusted host lane is intentionally narrow:

- runnable `process_stdio` packages only
- explicit `--source-language`
- generated local runtime stub files
- bounded smoke command output from `plugins init`

That keeps the public authoring contract honest while the broader executor
story is still evolving.

## What Is Not Promised

The public contract should not promise:

- internal `crates/app` helper APIs
- internal registry organization
- executor layout
- automatic self-evolution behavior

Those remain internal or experimental until proven durable.
