# External Authoring Contract

## Purpose

This document defines the public-facing authoring contract for LoongClaw
capability artifacts.

Its purpose is to let community authors build capability packages without
depending on internal crate layout.

## Core Thesis

The public SDK should be contract-first, package-first, and artifact-first.

LoongClaw should stabilize:

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

## What Is Not Promised

The public contract should not promise:

- internal `crates/app` helper APIs
- internal registry organization
- executor layout
- automatic self-evolution behavior

Those remain internal or experimental until proven durable.
