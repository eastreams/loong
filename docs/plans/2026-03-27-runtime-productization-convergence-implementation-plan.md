# Runtime Productization Convergence Implementation Plan

Date: 2026-03-27
Issue: #652
Related epics: #217, #292, #421
Stack base reviewed for merge: `fe87d347bc83c46dee47d8e78258643c2d8cb812`

## Goal

Turn the conclusions from `docs/design-docs/reference-runtime-comparison.md`
into an implementation order that closes the highest-value product gaps without
rewriting substrate that LoongClaw already has.

## Scope

In scope:

- task-shaped productization over the current session runtime
- discovery-first UX over the current external-skills runtime
- scoped, provenance-rich memory retrieval over the current canonical and
  staged memory stack
- docs, specs, and roadmap updates that keep future implementation aligned

Out of scope:

- Web UI
- full cron or service-runtime ownership in the first slice
- remote marketplace implementation in the first slice
- embedding-based semantic retrieval in the first slice

## Ordering Principle

The next slices should follow one rule:

> productize current substrate before adding new substrate

That means:

- do not add a second task engine before task UX exists
- do not add a new skill runtime before search and recommendation exist
- do not jump to embeddings before scoped provenance-rich retrieval exists

## Slice 0: Land the Durable Comparison Contract

Goal:

- make the convergence logic durable and repo-local before more implementation
  begins

Artifacts:

- `docs/design-docs/reference-runtime-comparison.md`
- `docs/plans/2026-03-27-runtime-productization-convergence-implementation-plan.md`
- `docs/product-specs/background-tasks.md`
- `docs/product-specs/skills-discovery.md`
- `docs/product-specs/memory-retrieval.md`
- roadmap and index updates

Validation:

- file existence checks
- markdown review
- repo verification commands still green

## Slice 1: Background Task Productization

Why first:

- this is the clearest step from "strong substrate" to "daily-usable runtime"
- it reuses the most mature existing internals
- it does not require committing to full scheduler ownership yet

Primary files:

- `crates/daemon/src/lib.rs`
- `crates/daemon/src/main.rs`
- `crates/app/src/tools/catalog.rs`
- `crates/app/src/tools/session.rs`
- `crates/app/src/session/repository.rs`
- new task-oriented CLI/helper module under `crates/daemon/src/`
- docs/product spec updates

Implementation shape:

1. add a task-shaped operator entrypoint on top of child sessions
2. translate task operations onto existing session tools
3. expose:
   - create
   - list
   - status
   - wait
   - events or follow
   - cancel
   - recover
4. surface approval-pending, blocked, and narrowed-tool state as task
   diagnostics
5. keep `session_id` as the runtime truth, but stop making it the primary user
   concept

Tests:

- task lifecycle round-trip over async delegate child sessions
- cancel and recover behavior
- approval-pending visibility
- task rendering over visible and hidden session boundaries

Out of scope for Slice 1:

- cron
- heartbeat
- daemon ownership
- service installation

## Slice 2: Skills Discovery-First UX

Why second:

- highest UX lift per added runtime complexity
- the managed runtime already exists
- this closes the largest operator friction gap in the current skill flow

Primary files:

- `crates/app/src/tools/external_skills.rs`
- `crates/daemon/src/skills_cli.rs`
- config/runtime helpers only if needed for curated registry support
- docs/product spec updates

Implementation shape:

1. add search/filter capability on top of `SkillDiscoveryInventory`
2. add recommendation ranking that prefers:
   - eligible
   - visible
   - non-shadowed
   - scope-preferred
   - bundled or curated sources when available
3. render explicit why-not diagnostics for:
   - blocked
   - ineligible
   - shadowed
   - manual-only invocation
4. return first-task guidance after install or inspect
5. preserve explicit install and invoke boundaries

Tests:

- search ranking across managed, user, and project scope
- shadowed-skill explanation
- ineligible-skill explanation
- first-use guidance rendering

Out of scope for Slice 2:

- blind remote auto-install
- dynamic provider tool registration per skill
- arbitrary code execution during install

## Slice 3: Scoped Memory Retrieval with Provenance

Why third:

- highest architecture sensitivity
- strongest interaction with identity and continuity boundaries
- easiest area to accidentally solve with the wrong shortcut

Primary files:

- `crates/app/src/memory/canonical.rs`
- `crates/app/src/memory/stage.rs`
- `crates/app/src/memory/orchestrator.rs`
- `crates/app/src/memory/sqlite.rs`
- operator CLI module for memory commands if introduced in the first slice
- docs/product spec updates

Implementation shape:

1. extend retrieval request construction to support explicit query and scopes
2. add local text retrieval before embedding-dependent retrieval
3. emit retrieved entries with explicit provenance fields such as:
   - scope
   - kind
   - source id
   - timestamp or freshness
   - injection reason
4. keep runtime-self and resolved runtime identity authoritative
5. expose operator-readable search and diagnostics surfaces

Tests:

- query-aware retrieval request construction
- scope isolation across session, user, agent, and workspace
- provenance rendering and stable serialization
- fail-open behavior when retrieval backend is unavailable

Out of scope for Slice 3:

- mandatory embeddings
- vector-only retrieval
- external memory vendor authority
- identity promotion from retrieved artifacts

## Verification Matrix

Repository-wide verification should remain mandatory after each landed slice:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Slice-specific verification should also exist before each PR is marked ready:

- focused unit tests for the changed surface
- operator-facing text output checks when CLI rendering changes
- docs and roadmap alignment review for any changed product contract

## Risk Notes

### Task UX Risk

Risk:

- accidentally building a parallel task runtime that drifts from session truth

Mitigation:

- task surface must be a translation layer over existing session substrate

### Skills UX Risk

Risk:

- discovery work drifting into ungoverned auto-install behavior

Mitigation:

- keep search and recommendation separate from install approval and invocation

### Memory Risk

Risk:

- retrieval shortcuts overriding identity or runtime-self boundaries

Mitigation:

- keep retrieval advisory
- preserve continuity lanes
- require explicit provenance on retrieved artifacts
