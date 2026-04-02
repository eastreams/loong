# Shared CLI Session Selector Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move reusable CLI `latest` session-selector resolution into one shared app-session helper and route both `chat` and `tasks` through it without changing surrounding caller-specific policies.

**Architecture:** Add one small helper surface in `crates/app/src/session/mod.rs` that owns the canonical selector token and the repository-backed `latest` lookup. Keep default-session handling, literal-session preservation, and caller-specific error messages in their existing surfaces.

**Tech Stack:** Rust, Cargo, `loongclaw-app`, `loongclaw` daemon integration tests, sqlite-backed session repository

---

### Task 1: Add the shared session-selector helper contract tests

**Files:**
- Modify: `crates/app/src/session/mod.rs`

**Step 1: Write the failing tests**

Add focused tests that:

1. seed multiple root sessions and verify the shared helper returns the newest resumable root
   session id
2. verify the shared helper returns `None` when no resumable root session exists

**Step 2: Run the focused app test to verify failure**

Run:

```bash
cargo test -p loongclaw-app latest_cli_session_selector --locked
```

Expected: FAIL before the shared helper exists.

### Task 2: Implement the shared helper

**Files:**
- Modify: `crates/app/src/session/mod.rs`

**Step 1: Add the minimal shared surface**

Implement:

1. a shared `LATEST_SESSION_SELECTOR` constant
2. a `latest_resumable_root_session_id(...)` helper that returns `CliResult<Option<String>>`

**Step 2: Re-run the focused app test**

Run:

```bash
cargo test -p loongclaw-app latest_cli_session_selector --locked
```

Expected: PASS.

### Task 3: Route chat through the shared helper

**Files:**
- Modify: `crates/app/src/chat.rs`
- Test: `crates/app/src/chat/latest_session_selector_tests.rs`

**Step 1: Replace the local selector token and repository lookup**

Update `chat.rs` so it:

1. imports the shared selector token
2. imports the shared lookup helper
3. preserves existing `CliSessionRequirement` behavior
4. keeps the existing chat-specific missing-session error wording

**Step 2: Run focused chat tests**

Run:

```bash
cargo test -p loongclaw-app cli_runtime_latest_session_selector --locked
```

Expected: PASS.

### Task 4: Route tasks through the shared helper

**Files:**
- Modify: `crates/daemon/src/tasks_cli.rs`
- Test: `crates/daemon/tests/integration/tasks_cli.rs`

**Step 1: Replace the local selector token and repository lookup**

Update `tasks_cli.rs` so it:

1. imports the shared selector token
2. imports the shared lookup helper
3. keeps `tasks`-specific normalization and error text local

**Step 2: Run focused daemon tests**

Run:

```bash
cargo test -p loongclaw --test integration tasks_ --locked
```

Expected: PASS.

### Task 5: Verify the cross-surface contract

**Files:**
- Modify only if test output exposes a real regression

**Step 1: Re-run existing latest-selector coverage**

Run:

```bash
cargo test -p loongclaw --test integration latest_session_selector --locked
```

Expected: PASS.

**Step 2: Run repo-wide verification**

Run:

```bash
git diff --check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

Expected: all green.

### Task 6: Prepare GitHub delivery artifacts

**Files:**
- Modify: issue and PR bodies only

**Step 1: Reuse issue #809**

Describe:

1. the duplication problem across `chat` and `tasks`
2. the scoped shared-helper solution
3. the explicit non-goals around broader CLI abstraction

**Step 2: Open the PR linked to that issue**

Include:

1. the ownership-boundary rationale
2. the focused validation evidence
3. the scope boundary that this change does not unify all session-hint policy
