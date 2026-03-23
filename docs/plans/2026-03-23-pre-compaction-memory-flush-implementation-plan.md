# Pre-Compaction Memory Flush Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a deterministic pre-compaction durable memory flush that exports advisory session continuity into a workspace memory file right before LoongClaw compacts context.

**Architecture:** Reuse the existing SQLite summary checkpoint machinery instead of adding a second summarization path or a hidden LLM turn. Wire a small flush helper into the existing compaction gate, keep the flushed record explicitly advisory, and deduplicate repeated exports with a stable content hash.

**Tech Stack:** Rust, existing `conversation` runtime hooks, existing SQLite summary checkpoint/context snapshot logic, filesystem writes under the configured safe file root, focused unit/integration tests.

---

### Task 1: Add the failing tests for deterministic durable flush behavior

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/memory/tests.rs`

**Step 1: Write the failing tests**

Add tests that cover:
- flush runs when compaction is triggered
- flush does not run when compaction is skipped
- repeated compaction attempts with unchanged content do not duplicate the durable note
- flushed content is labeled as advisory and does not look like runtime identity

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-app pre_compaction
```

Expected:
- at least one new test fails because no durable flush exists yet

**Step 3: Commit the red state only after confirming failure locally**

Do not commit implementation code in this task.

### Task 2: Add helper plumbing for pre-compaction export

**Files:**
- Modify: `crates/app/src/memory/mod.rs`
- Create: `crates/app/src/memory/durable_flush.rs`

**Step 1: Reuse the existing safe workspace root gate**

Use the existing configured `tools.file_root` as the opt-in safe workspace root
for durable export. Do not add a second toggle unless implementation pressure
proves it necessary.

**Step 2: Add deterministic helper**

Create a helper that:
- resolves the safe workspace root
- materializes the durable export content from existing summary/checkpoint data
- writes to `memory/YYYY-MM-DD.md`
- stamps metadata needed for dedupe
- treats the flushed note as advisory only

**Step 3: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app durable_flush
```

Expected:
- helper-level tests pass

### Task 3: Wire the helper into the compaction gate

**Files:**
- Modify: `crates/app/src/conversation/turn_coordinator.rs`

**Step 1: Hook immediately before runtime compaction**

In `maybe_compact_context(...)`:
- keep the existing compaction eligibility logic
- if compaction will run, invoke the pre-compaction durable flush helper first
- preserve existing fail-open / fail-closed behavior intentionally

**Step 2: Keep scope tight**

Do not alter unrelated turn finalization paths.
Do not add retrieval or read-path logic in this task.

**Step 3: Run focused tests**

Run:

```bash
cargo test -p loongclaw-app pre_compaction
```

Expected:
- new compaction-path tests pass

### Task 4: Document the new behavior

**Files:**
- Modify: `docs/product-specs/runtime-self-continuity.md`
- Modify: `docs/product-specs/memory-profiles.md`

**Step 1: Document the new runtime seam**

Describe:
- when the pre-compaction flush runs
- that it writes advisory durable recall only
- that it does not override runtime identity

**Step 2: Keep docs aligned with `#421`, `#440`, and `#468`**

Do not over-promise retrieval or search behavior.

### Task 5: Verify the full touched surface

**Files:**
- Verify only

**Step 1: Run formatting**

```bash
cargo fmt --all
cargo fmt --all --check
```

**Step 2: Run targeted tests**

```bash
cargo test -p loongclaw-app pre_compaction
cargo test -p loongclaw-app durable_flush
```

**Step 3: Run broader app verification**

```bash
cargo test -p loongclaw-app --lib
cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings
```

**Step 4: Run workspace verification if the stacked base is still valid**

Preferred:

```bash
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

If plain `dev` remains blocked by unrelated open stacked prerequisites, rerun the same verification on the known-good stacked verification worktree and record that evidence explicitly.

### Task 6: Commit the implementation cleanly

**Files:**
- Modify only the files touched by this plan

**Step 1: Inspect staged scope**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

**Step 2: Commit**

```bash
git add docs/plans/2026-03-23-pre-compaction-memory-flush-implementation-plan.md
git add crates/app/src/config/conversation.rs
git add crates/app/src/config/mod.rs
git add crates/app/src/memory/mod.rs
git add crates/app/src/memory/durable_flush.rs
git add crates/app/src/conversation/turn_coordinator.rs
git add crates/app/src/conversation/tests.rs
git add crates/app/src/memory/tests.rs
git add docs/product-specs/runtime-self-continuity.md
git add docs/product-specs/memory-profiles.md
git commit -m "feat(app): flush durable memory before compaction"
```
