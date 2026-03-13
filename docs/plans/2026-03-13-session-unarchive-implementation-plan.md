# Session Unarchive Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a truthful `session_unarchive` primitive that restores visible archived terminal
sessions to default `sessions_list` inventory and updates archive-state derivation to use the latest
archive control event.

**Architecture:** Keep archive lifecycle event-sourced. Introduce `session_unarchived` as a durable
control event, teach repository summary queries to derive archive state from the latest archive
control event (`session_archived` or `session_unarchived`), and expose `session_unarchive` with the
same single-target / batch / `dry_run` mutation pattern used elsewhere in the session tool surface.

**Tech Stack:** Rust, rusqlite, serde_json, sqlite-backed session repository, cargo test

---

### Task 1: Document the reversible archive design

**Files:**
- Create: `docs/plans/2026-03-13-session-unarchive-design.md`
- Create: `docs/plans/2026-03-13-session-unarchive-implementation-plan.md`

**Step 1: Write the design doc**

Capture:

- why `session_unarchive` is the next truthful primitive
- why `session_close` is still rejected
- why latest archive control event must win
- why repository derivation should prefer event `id` over `ts`

**Step 2: Write the implementation plan**

Break implementation into TDD-sized tasks with exact files and commands.

**Step 3: Commit**

```bash
git add docs/plans/2026-03-13-session-unarchive-design.md docs/plans/2026-03-13-session-unarchive-implementation-plan.md
git commit -m "docs(plans): design session unarchive tool"
```

### Task 2: Add the first failing `session_unarchive` test

**Files:**
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing test**

Add a test proving that:

- a visible archived terminal child can be unarchived
- the returned payload includes `unarchive_action`
- the target becomes non-archived in the returned inspection payload
- `session_status` still reports the target and now marks it unarchived

**Step 2: Run test to verify it fails**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_unarchive_restores_archived_terminal_visible_session -- --nocapture --test-threads=1
```

Expected: FAIL because `session_unarchive` is not implemented.

**Step 3: Write minimal implementation**

Implement the smallest code path needed for single-target unarchive behavior in the session tool
layer.

**Step 4: Run test to verify it passes**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/session.rs
git commit -m "feat(app): add single-session unarchive flow"
```

### Task 3: Add the failing repository regression for archive-state derivation

**Files:**
- Modify: `crates/app/src/session/repository.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing tests**

Add tests proving that:

- a session archived and then unarchived reports `archived_at = null`
- `sessions_list` default output includes that session again
- `include_archived=true` still shows genuinely archived sessions
- the latest archive control event wins even when both archive and unarchive events exist

**Step 2: Run tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_unarchive -- --nocapture --test-threads=1
```

Expected: FAIL because repository summary queries still only read `session_archived`.

**Step 3: Write minimal implementation**

Update repository summary loading to derive archive state from the latest archive control event,
preferring event `id` ordering.

**Step 4: Run tests to verify they pass**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/session/repository.rs crates/app/src/tools/session.rs
git commit -m "feat(app): derive archive state from latest control event"
```

### Task 4: Add batch and `dry_run` unarchive coverage

**Files:**
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing tests**

Add tests proving that:

- batch `session_unarchive` supports `session_ids`
- `dry_run=true` previews mixed results without mutation
- already-visible non-archived sessions are classified as `skipped_not_archived`
- non-terminal or hidden sessions classify correctly

**Step 2: Run tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_unarchive_batch -- --nocapture --test-threads=1
```

Expected: FAIL on missing batch behavior.

**Step 3: Write minimal implementation**

Extend the existing mutation helper pattern used by `session_archive` to implement batch
`session_unarchive`.

**Step 4: Run tests to verify they pass**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/session.rs
git commit -m "feat(app): add batch session unarchive support"
```

### Task 5: Update the tool surface and product docs

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/provider/mod.rs`
- Modify: `docs/product-specs/index.md`
- Modify: `docs/roadmap.md`

**Step 1: Write the failing tests**

Add or update tests proving that:

- `session_unarchive` appears in the root tool surface
- delegated child views still do not gain it
- provider request bodies expose its schema

**Step 2: Run tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_unarchive -- --nocapture --test-threads=1
```

Expected: FAIL on catalog / provider assertions.

**Step 3: Write minimal implementation**

Add tool catalog entries, schema definition, visibility wiring, and product-doc updates that match
the implemented behavior.

**Step 4: Run tests to verify they pass**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs crates/app/src/provider/mod.rs docs/product-specs/index.md docs/roadmap.md
git commit -m "feat(app): expose session unarchive tool surface"
```

### Task 6: Run full verification

**Files:**
- No code changes expected

**Step 1: Run focused tests**

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_unarchive -- --nocapture --test-threads=1
```

Expected: PASS

**Step 2: Run package test suite**

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app -- --nocapture --test-threads=1
```

Expected: PASS

**Step 3: Run daemon compile verification**

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-daemon --no-run
```

Expected: PASS

**Step 4: Run formatting**

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all
```

Expected: no diff

**Step 5: Inspect and commit any final cleanups**

```bash
git status --short
git diff --cached --name-only
git diff --cached
```
