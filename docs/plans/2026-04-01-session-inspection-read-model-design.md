# Session Inspection Read Model Extraction Design

**Date:** 2026-04-01

**Tracking Issue:** `loongclaw-ai/loongclaw#776`

**Goal:** Extract the session inspection read-side model from `crates/app/src/tools/session.rs` into a dedicated `session` domain module so observation assembly, delegate lifecycle inference, recovery synthesis, and inspection payload shaping no longer live inside the tool surface.

## Problem

`crates/app/src/tools/session.rs` currently owns several different responsibilities at once:

- tool request parsing and output wiring
- repository-backed observation loading
- delegate lifecycle inference
- missing terminal outcome interpretation
- recovery projection
- inspection JSON shaping

That shape was acceptable while the feature set was still narrow, but it is now a structural liability.

The main issue is not line count by itself.
The issue is that the tool surface owns domain rules that should be reusable and testable without going through the session tool entrypoint.

That has three concrete costs:

- read-side session behavior is harder to reuse from future operator or SDK surfaces
- `tools/session.rs` review scope stays wide even for narrow domain fixes
- read-side logic drifts independently from the write-side operator seams that were already extracted into `operator::delegate_runtime`

## Current State

The current read-side inspection flow spans these seams:

- `SessionRepository::load_session_observation(...)` returns the raw repository snapshot
- `tools/session.rs` builds `SessionInspectionSnapshot`
- `tools/session.rs` loads delegate lifecycle events
- `tools/session.rs` derives delegate lifecycle state
- `tools/session.rs` synthesizes missing recovery information
- `tools/session.rs` emits the final JSON payload

This means the session tool is acting as both a transport surface and a domain read model owner.

## Proposed Cut

Create a new internal module:

- `crates/app/src/session/inspection.rs`

Move the session inspection read-side model into that module.

The new module should own:

- read-side snapshot structs used by inspection
- repository-backed observation loading helpers
- delegate lifecycle derivation
- terminal outcome state derivation
- missing-recovery interpretation
- inspection JSON projection

`tools/session.rs` should keep only:

- tool payload parsing
- tool-level option defaults and validation
- action orchestration for list, inspect, wait, recover, cancel, archive
- response formatting specific to the tool surface

## Proposed API Shape

The first iteration should stay intentionally narrow.

I recommend exposing only a small internal API from `session::inspection`:

- `load_session_observation_snapshot(...) -> Result<SessionObservationSnapshot, String>`
- `session_inspection_payload(...) -> Value`
- `session_state_is_terminal(...) -> bool`

The new module can keep additional helper structs and helper functions private unless another caller already needs them.

This is the smallest extraction that still moves ownership cleanly.

## Behavioral Rules To Preserve

This cut should not change user-visible session inspection behavior.

The extraction must preserve:

- the current inspection JSON field names
- the current delegate lifecycle payload shape
- the current recovery payload shape
- the current newest-recovery-event-over-`last_error` precedence
- the current rule that only delegate children expose delegate lifecycle
- the current rule that missing terminal outcomes only synthesize recovery for terminal sessions

## Testing Strategy

The safest path is to keep the outer session-tool integration tests green while adding a few focused tests near the new module boundary.

That should include:

- preserving current `session_status_*` recovery expectations
- preserving current delegate lifecycle payload expectations
- adding direct unit coverage around the new inspection module for:
  - missing terminal outcome state derivation
  - recovery attachment rules
  - delegate lifecycle derivation from event history

The goal is not to redesign the behavior.
The goal is to make the behavior locally testable under its true owner.

## Alternatives Considered

### 1. Full typed read model plus separate serializer

This would be architecturally cleaner in the long run.

I am not choosing it now because it would widen the diff substantially.
It would mix ownership extraction with payload redesign risk.

### 2. Keep logic in `tools/session.rs` and only add comments or tests

This is the cheapest short-term path.

I am not choosing it because it does not solve the ownership problem.
It keeps future SDK-facing reuse blocked on the tool surface.

### 3. Extract both read-side inspection and write-side session mutations together

This would create a broader `session` service layer in one pass.

I am not choosing it because it is too wide for the next stacked PR.
It would blur whether regressions come from read-side extraction or write-side refactoring.

## Why This Cut

This cut is the best next step after the recent delegate runtime and recovery work.

The write-side operator seam already exists.
The recovery helper contract is now directly fenced.
The largest remaining structural concentration is the read-side inspection model in `tools/session.rs`.

Extracting that read-side seam now gives the highest structural payoff for the lowest behavior risk.

## Scope

In scope:

- add `session::inspection`
- move session inspection read-side types and helpers out of `tools/session.rs`
- preserve existing inspection JSON behavior
- add direct tests for the extracted read-side seam

Out of scope:

- redesign session inspection payloads
- change repository schemas
- change delegate child write-side creation
- extract session mutation services for cancel, recover, or archive
- expose a new public SDK surface in this pass

## Validation Plan

This work should be validated with:

- focused tests for the new inspection module
- existing `tools/session.rs` integration tests that cover delegate lifecycle and recovery inspection
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

## Expected Outcome

After this lands:

- `tools/session.rs` becomes a thinner tool surface
- session inspection behavior has a clear domain owner
- future session inspection reuse from operator or SDK surfaces becomes easier
- the next refactor can target write-side session actions without mixing in read-side inspection rules
