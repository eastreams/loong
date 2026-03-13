# Session Archive Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a truthful `session_archive` operator primitive that retires visible terminal sessions from default `sessions_list` inventory while preserving direct inspection and history.

**Architecture:** Archive is a separate inventory overlay, not a new execution state. The implementation adds durable archive metadata to session summaries, exposes a new `session_archive` tool with single-target and batch behavior, and teaches `sessions_list` to exclude archived sessions by default while surfacing archive metadata when requested.

**Tech Stack:** Rust, rusqlite, serde_json, sqlite-backed session repository, cargo test

---

### Task 1: Add the first failing session-tool test

**Files:**
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing test**

Add a test proving that:

- a terminal visible child can be archived
- the returned payload includes `archive_action`
- `session_status` still reports the target and marks it archived

**Step 2: Run test to verify it fails**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_archive_archives_terminal_visible_session -- --nocapture --test-threads=1
```

Expected: FAIL because `session_archive` is not implemented.

**Step 3: Write minimal implementation**

Implement the smallest code path needed for single-target archive behavior.

**Step 4: Run test to verify it passes**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/session.rs crates/app/src/session/repository.rs crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs crates/app/src/provider/mod.rs
git commit -m "feat(app): add single-session archive flow"
```

### Task 2: Add list filtering and archive metadata coverage

**Files:**
- Modify: `crates/app/src/session/repository.rs`
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/provider/mod.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests proving that:

- `sessions_list` excludes archived sessions by default
- `sessions_list` returns archived sessions when `include_archived=true`
- returned summaries expose `archived` and `archived_at`
- provider/tool-catalog output now includes `session_archive`

**Step 2: Run tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_archive -- --nocapture --test-threads=1
```

Expected: FAIL on the new list/catalog assertions.

**Step 3: Write minimal implementation**

Implement:

- archive metadata loading in repository summaries
- `include_archived` parsing and filtering
- tool catalog / provider schema exposure for `session_archive`

**Step 4: Run tests to verify they pass**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/session/repository.rs crates/app/src/tools/session.rs crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs crates/app/src/provider/mod.rs
git commit -m "feat(app): hide archived sessions from default listings"
```

### Task 3: Add batch archive and rejection-path coverage

**Files:**
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing tests**

Add tests proving that:

- batch `session_archive` supports `session_ids`
- `dry_run=true` previews mixed outcomes without mutation
- already archived sessions are classified separately
- non-terminal or hidden sessions are rejected/classified correctly

**Step 2: Run tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_archive_batch -- --nocapture --test-threads=1
```

Expected: FAIL on missing batch behavior.

**Step 3: Write minimal implementation**

Extend the mutation helper pattern used by `session_cancel` / `session_recover` to archive batching.

**Step 4: Run tests to verify they pass**

Run the same command and confirm PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/session.rs
git commit -m "feat(app): add batch session archive support"
```

### Task 4: Update product docs

**Files:**
- Modify: `docs/product-specs/index.md`
- Modify: `docs/roadmap.md`

**Step 1: Write the doc updates**

Document:

- `session_archive` in the accepted root tool surface
- default `sessions_list` archive exclusion
- archive limitations and non-goals

**Step 2: Verify docs are accurate**

Re-read the design doc and match wording against actual implementation behavior.

**Step 3: Commit**

```bash
git add docs/product-specs/index.md docs/roadmap.md
git commit -m "docs: describe session archive behavior"
```

### Task 5: Run full verification

**Files:**
- No code changes expected

**Step 1: Run focused tests**

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_archive -- --nocapture --test-threads=1
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
