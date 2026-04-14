# Runtime Productization Convergence Implementation Plan

Date: 2026-04-14
Public scope: LoongClaw-native productization order for workflow, task, skill,
and memory surfaces

## Goal

Close the highest-value operator-facing product gaps by building on runtime
substrate that LoongClaw already ships.

## Scope

In scope:

- governed workflow surface above the current session runtime
- task-shaped productization over the current child-session substrate
- discovery-first UX over the current external-skills runtime
- scoped, provenance-rich memory retrieval over the current canonical and
  staged memory stack
- public docs, specs, and roadmap updates that keep future implementation
  aligned

Out of scope:

- Web UI implementation
- full cron or service-runtime ownership in the first slice
- remote marketplace implementation in the first slice
- mandatory embedding-based retrieval in the first slice
- a second async scheduler beside child-session execution

## Ordering Principle

The next slices should follow one rule:

> productize current substrate before adding new substrate

## Slice 0: Public contract alignment

Goal:

- make the LoongClaw-native direction durable in public specs and roadmap

Artifacts:

- `docs/product-specs/governed-workflows.md`
- `docs/product-specs/background-tasks.md`
- `docs/product-specs/skills-discovery.md`
- `docs/product-specs/memory-retrieval.md`
- `docs/product-specs/runtime-self-continuity.md`
- roadmap and index updates

Validation:

- markdown review
- repo verification commands still green
- public wording stays LoongClaw-centric

## Slice 1: Governed workflow surface and task runtime maturation

Why first:

- this is the clearest step from strong substrate to daily-usable long-running
  work
- it reuses the most mature existing internals
- it avoids inventing a second scheduler

Primary files:

- `crates/contracts/src/workflow_types.rs`
- `crates/daemon/src/tasks_cli.rs`
- `crates/app/src/tools/session.rs`
- `crates/app/src/session/repository.rs`
- `crates/daemon/src/control_plane_server.rs`
- related product spec updates

Implementation shape:

1. add a governed workflow read model above child-session truth
2. make workflow phases explicit and inspectable
3. keep task operations rooted in existing session/task substrate
4. surface lineage, approval attention, and runtime narrowing as workflow-aware
   diagnostics
5. keep `session_id` and lineage canonical even when task language becomes the
   primary operator surface

Tests:

- workflow and task lifecycle round-trips
- cancel and recover behavior
- approval and narrowing visibility
- status rendering over visible and hidden boundaries

## Slice 2: Skills discovery-first UX

Why second:

- highest UX lift per added runtime complexity
- the managed runtime already exists
- this closes a major operator friction gap without widening trust boundaries

Primary files:

- `crates/app/src/tools/external_skills.rs`
- `crates/daemon/src/skills_cli.rs`
- related product spec updates

Implementation shape:

1. add search and recommendation on top of the current discovery inventory
2. render why-not diagnostics for blocked, shadowed, or ineligible candidates
3. return first-task guidance after inspect or install
4. preserve explicit install and invoke boundaries

Tests:

- ranking across managed, user, and project scope
- shadowed-skill explanation
- ineligible-skill explanation
- first-use guidance rendering

## Slice 3: Memory retrieval and continuity refinement

Why third:

- strongest interaction with identity and continuity boundaries
- easiest area to solve with the wrong shortcut if order slips

Primary files:

- `crates/app/src/memory/*`
- `crates/app/src/tools/memory_tools.rs`
- related product spec updates

Implementation shape:

1. deepen derived-memory ranking and provenance
2. preserve advisory retrieval boundaries
3. keep runtime-self and resolved runtime identity authoritative
4. optionally layer workflow-aware durable memory only as advisory context

Tests:

- query-aware retrieval request construction
- scope isolation and provenance rendering
- workflow-aware memory remaining advisory
- fail-open behavior when optional retrieval helpers are unavailable

## Verification Matrix

Repository-wide verification should remain mandatory after each landed slice:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

## Risks

### Workflow truth drift

Risk:

- workflow productization could create a second state model beside session truth

Mitigation:

- session lineage remains canonical
- workflow state is a read model above existing runtime truth

### Continuity drift

Risk:

- workflow or task journals could start acting like identity authority

Mitigation:

- runtime-self continuity remains authoritative
- workflow memory remains advisory
