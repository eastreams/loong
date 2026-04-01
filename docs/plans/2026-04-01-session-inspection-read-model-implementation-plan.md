# Session Inspection Read Model Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Tracking Issue:** `loongclaw-ai/loongclaw#776`

**Goal:** Extract the session inspection read-side model from `tools/session.rs` into `session::inspection` without changing the existing inspection payload contract.

**Architecture:** Introduce a dedicated `session::inspection` internal module that owns repository-backed observation loading, delegate lifecycle derivation, recovery attachment, and inspection JSON assembly. Keep the session tool surface as an orchestration and request/response adapter layer. Preserve current behavior first; do not redesign the payload shape in this pass.

**Tech Stack:** Rust, `serde_json`, existing `SessionRepository`, existing `session::recovery` helpers, existing session tool integration tests, workspace cargo verification.

---

### Task 1: Add the new session inspection module skeleton

**Files:**
- Modify: `crates/app/src/session/mod.rs`
- Create: `crates/app/src/session/inspection.rs`

**Step 1: Create the new module export**

Add:

```rust
#[cfg(feature = "memory-sqlite")]
pub mod inspection;
```

**Step 2: Create the destination module with compile-only placeholders**

Start with:

```rust
#[cfg(feature = "memory-sqlite")]
use serde_json::Value;
```

and the first extracted structs and function signatures.

**Step 3: Run compile-focused check**

Run:

```bash
cargo test -p loongclaw-app --lib session::recovery -- --nocapture
```

Expected:
- compile still succeeds while the new module is introduced incrementally

### Task 2: Move the read-side snapshot and observation-loading types

**Files:**
- Modify: `crates/app/src/session/inspection.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Move these structs into `session::inspection`**

Move:

- `SessionInspectionSnapshot`
- `SessionObservationSnapshot`

**Step 2: Move repository-backed observation loading helpers**

Move:

- the helper that loads the observation snapshot from `SessionRepository`
- the helper that loads delegate lifecycle events for delegate children

Keep function names descriptive and domain-owned.

**Step 3: Update `tools/session.rs` call sites to use the new module**

Replace local calls with imports from `crate::session::inspection`.

**Step 4: Run targeted inspection tests**

Run:

```bash
cargo test -p loongclaw-app session_status -- --nocapture
```

Expected:
- existing session status tests still pass after the move

### Task 3: Move delegate lifecycle derivation into the new module

**Files:**
- Modify: `crates/app/src/session/inspection.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Move the read-side lifecycle structs**

Move:

- `SessionDelegateLifecycleRecord`
- `SessionDelegateStalenessRecord`
- `SessionDelegateCancellationRecord`

**Step 2: Move the derivation helpers**

Move:

- `session_delegate_lifecycle_at(...)`
- `session_delegate_staleness_at(...)`
- JSON helpers for lifecycle, staleness, and cancellation

**Step 3: Preserve current JSON shape exactly**

Do not rename fields.
Do not add or remove payload keys.

**Step 4: Run focused lifecycle tests**

Run:

```bash
cargo test -p loongclaw-app session_delegate_lifecycle -- --nocapture
```

Expected:
- lifecycle-specific tests stay green

### Task 4: Move terminal outcome and recovery attachment rules

**Files:**
- Modify: `crates/app/src/session/inspection.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Move the inspection payload assembly helpers**

Move:

- `session_state_is_terminal(...)`
- `session_terminal_outcome_state(...)`
- `session_terminal_outcome_missing_reason(...)`
- `session_inspection_payload(...)`

**Step 2: Keep recovery synthesis delegated to `session::recovery`**

Use:

- `observe_missing_recovery(...)`
- `recovery_json(...)`

Do not duplicate recovery logic in the new module.

**Step 3: Add direct tests near the new owner**

Add tests for:

- terminal sessions with missing terminal outcome attach recovery
- non-terminal sessions do not attach recovery
- missing terminal outcome reason tracks the recovery kind

**Step 4: Run targeted recovery-facing inspection tests**

Run:

```bash
cargo test -p loongclaw-app session_status_synthesizes_recovery -- --nocapture
```

Expected:
- recovery-facing inspection behavior remains unchanged

### Task 5: Thin `tools/session.rs` down to the tool surface

**Files:**
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Remove the extracted duplicate definitions from the tool module**

Delete moved structs and helper functions from `tools/session.rs`.

**Step 2: Replace them with narrow imports from `session::inspection`**

Keep only tool-surface orchestration logic in the file.

**Step 3: Re-read the file for remaining structural mixing**

Specifically check that `tools/session.rs` no longer owns:

- observation loading details
- lifecycle derivation details
- inspection payload assembly

### Task 6: Run crate-level verification

**Files:**
- Verify only

**Step 1: Run formatting**

```bash
cargo fmt --all -- --check
```

**Step 2: Run strict lint**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

**Step 3: Run workspace tests**

```bash
cargo test --workspace --locked
```

**Step 4: Run all-feature workspace tests**

```bash
cargo test --workspace --all-features --locked
```

Expected:
- all verification passes
- if a baseline-only failure appears, stop and document it explicitly

### Task 7: Commit and deliver

**Files:**
- Only the files touched by this extraction

**Step 1: Inspect staged scope**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Expected:
- only inspection extraction files and supporting tests/docs are staged

**Step 2: Commit**

Use a narrow message such as:

```bash
git commit -m "refactor(app): extract session inspection read model"
```

**Step 3: Push and open a stacked PR**

Use:

- issue-first workflow
- English GitHub text
- `--body-file` for multi-line markdown
- explicit stacked-branch note
