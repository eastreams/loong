# Runtime Productization Direction

> Detailed comparative analysis is archived in the internal knowledge base.
> This public page keeps only the LoongClaw-specific productization direction.

Date: 2026-04-14
Public focus: LoongClaw-native workflow, task, skills, and memory productization

## Scope

This page captures the public implementation direction for the next
LoongClaw-facing operator surfaces.

It focuses on:

- governed workflows and long-running work
- background task productization on top of session runtime
- discovery-first skills UX
- scoped, provenance-rich memory retrieval

It does not publish raw external project comparison or broader internal
sequencing rationale.

## Current LoongClaw Baseline

LoongClaw already ships meaningful substrate in the areas that matter most for
long-running work:

### Session and task substrate

The current runtime already has:

- child-session execution through `delegate_async`
- operator-facing task controls through `loong tasks`
- session lineage and workflow metadata in session inspection
- recovery, approval, and tool-policy surfaces rooted in session truth

### Skills substrate

The current runtime already has:

- managed external-skills lifecycle surfaces
- discovery inventory and scope-aware visibility
- explicit governance around install, inspect, invoke, and removal

### Memory substrate

The current runtime already has:

- canonical history and staged memory vocabulary
- operator-facing `session_search`
- operator-facing `memory_search` and `memory_get`
- runtime-self continuity boundaries that keep identity authority separate from
  advisory durable memory

## Direction

The next public slices should follow one rule:

> productize current substrate before adding new substrate.

That means:

- workflows should build on current session and task truth
- background-task UX should stay truthful to child sessions
- skills UX should improve discovery and recommendation before new runtimes are
  added
- memory retrieval should deepen provenance and ranking before any stronger
  retrieval backend becomes mandatory

## Public Productization Order

### 1. Governed workflows and background tasks

The first productization priority is a truthful workflow surface above the
current session runtime.

Public contract anchors:

- [Governed Workflows](../product-specs/governed-workflows.md)
- [Background Tasks](../product-specs/background-tasks.md)
- [Local Product Control Plane](../product-specs/local-product-control-plane.md)
- [Runtime-Self Continuity](../product-specs/runtime-self-continuity.md)

### 2. Skills discovery-first UX

The second priority is making the existing external-skills runtime easier to
discover and adopt without weakening governance.

Public contract anchor:

- [Skills Discovery](../product-specs/skills-discovery.md)

### 3. Scoped memory retrieval with stronger derived-memory layering

The third priority is to deepen the shipped retrieval surface while preserving
continuity boundaries.

Public contract anchors:

- [Memory Retrieval](../product-specs/memory-retrieval.md)
- [Memory Profiles](../product-specs/memory-profiles.md)
- [Runtime-Self Continuity](../product-specs/runtime-self-continuity.md)

## Non-Goals

This direction should not be read as support for:

- a second task scheduler beside child-session execution
- public hosted workflow control planes by default
- ungoverned auto-install behavior for skills
- identity promotion from durable memory or workflow journals
- adapter-owned workflow truth in tmux, HUD, or browser-only layers
