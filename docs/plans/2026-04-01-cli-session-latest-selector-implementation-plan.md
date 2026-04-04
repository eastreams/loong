# CLI Session Latest Selector Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `--session latest` support for `loong ask` and `loong chat` without changing the
existing implicit-default or explicit-session-id behavior.

**Architecture:** Keep selector semantics in the CLI runtime layer and add one repository helper
that returns the latest resumable root session using existing session summary metadata and legacy
fallback compatibility.

**Tech Stack:** Rust, rusqlite, clap, existing app and daemon integration tests

---

## Task 1: Add the failing repository selection tests

**Files:**
- Modify: `crates/app/src/session/repository.rs`

**Step 1: Write a failing test for the normal selector rule**

Add a repository test that creates:

- one archived root session with turns
- one delegate-child session with newer timestamps
- one empty root session without turns
- one eligible root session

Assert that the new helper returns the eligible root session id.

**Step 2: Write a failing test for legacy compatibility**

Add a repository test that creates:

- one concrete root session with older activity
- one legacy turn-only root session with newer activity

Assert that the new helper returns the legacy root session id.

**Step 3: Run the focused repository tests and verify red**

Run:

```bash
cargo test -p loongclaw-app latest_resumable_root --locked
```

Expected: failing because the helper does not exist yet.

## Task 2: Add the failing CLI runtime tests

**Files:**
- Modify: `crates/app/src/chat.rs`

**Step 1: Write a failing runtime resolution test**

Use the existing chat test memory helpers to seed session history and assert that
`initialize_cli_turn_runtime_with_loaded_config()` resolves `Some("latest")` to the expected
concrete session id.

**Step 2: Write a failing no-match test**

Assert that `Some("latest")` returns a clear error when no eligible resumable root session exists.

**Step 3: Run the focused chat tests and verify red**

Run:

```bash
cargo test -p loongclaw-app latest_session --locked
```

Expected: failing because selector resolution is not implemented yet.

## Task 3: Add the failing daemon CLI parsing test

**Files:**
- Modify: `crates/daemon/tests/integration/mod.rs`
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Add a parsing test for `ask --session latest`**

Assert that clap parsing accepts `latest` as the `session` value for `ask`.

**Step 2: Add a parsing test for `chat --session latest`**

Assert that clap parsing accepts `latest` as the `session` value for `chat`.

**Step 3: Run the focused daemon parsing tests and verify red/green boundary**

Run:

```bash
cargo test -p loongclaw-daemon latest_session_selector --locked
```

Expected: likely already green at the parsing layer, which confirms no parser change is required.

## Task 4: Implement the repository helper

**Files:**
- Modify: `crates/app/src/session/repository.rs`

**Step 1: Add one public helper for selector resolution**

Add a method that returns the latest resumable root `SessionSummaryRecord` or `None`.

**Step 2: Reuse existing summary semantics**

The helper should:

- include concrete session summaries
- include eligible legacy turn-only root sessions
- exclude archived sessions
- exclude delegate-child sessions
- exclude sessions with `turn_count == 0`
- keep deterministic newest-first ordering

**Step 3: Re-run the focused repository tests**

Run:

```bash
cargo test -p loongclaw-app latest_resumable_root --locked
```

Expected: green.

## Task 5: Implement selector-aware CLI session resolution

**Files:**
- Modify: `crates/app/src/chat.rs`

**Step 1: Split literal and selector handling**

Keep the existing trim/default logic, then add a selector-aware resolver that can use the loaded
config and memory runtime.

**Step 2: Add `latest` handling**

When the trimmed value equals `latest`, resolve the newest eligible root session through the new
repository helper.

**Step 3: Preserve existing behavior**

Confirm:

- omitted `--session` still yields `default`
- concurrent host still requires an explicit concrete or selector value
- explicit session ids still pass through unchanged

**Step 4: Re-run the focused chat tests**

Run:

```bash
cargo test -p loongclaw-app latest_session --locked
```

Expected: green.

## Task 6: Confirm daemon CLI parsing coverage

**Files:**
- Modify: `crates/daemon/tests/integration/mod.rs`
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Run the focused daemon tests**

Run:

```bash
cargo test -p loongclaw-daemon latest_session_selector --locked
```

Expected: green.

**Step 2: Keep scope tight**

Do not change clap flag structure unless the tests prove it is necessary.

## Task 7: Run focused and broad verification

**Files:**
- Modify: none unless verification exposes a necessary fix

**Step 1: Run focused app and daemon tests**

Run:

```bash
cargo test -p loongclaw-app latest_resumable_root --locked
cargo test -p loongclaw-app latest_session --locked
cargo test -p loongclaw-daemon latest_session_selector --locked
```

**Step 2: Run broader relevant coverage**

Run:

```bash
cargo test -p loongclaw-app chat --locked
cargo test -p loongclaw-app session::repository --locked
```

**Step 3: Run formatting and workspace verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

Expected: all green, or any unrelated baseline failure must be explicitly investigated and
separated from this feature before claiming readiness.

## Task 8: Prepare clean GitHub delivery

**Files:**
- Modify: GitHub artifacts through `gh`, not repository files

**Step 1: Inspect isolated changes**

Run:

```bash
git status --short
git diff -- crates/app/src/chat.rs crates/app/src/session/repository.rs crates/daemon/tests/integration/mod.rs crates/daemon/tests/integration/cli_tests.rs docs/plans/2026-04-01-cli-session-latest-selector-design.md docs/plans/2026-04-01-cli-session-latest-selector-implementation-plan.md
```

**Step 2: Commit with a scoped message**

Run:

```bash
git add crates/app/src/chat.rs crates/app/src/session/repository.rs crates/daemon/tests/integration/mod.rs crates/daemon/tests/integration/cli_tests.rs docs/plans/2026-04-01-cli-session-latest-selector-design.md docs/plans/2026-04-01-cli-session-latest-selector-implementation-plan.md
git commit -m "Add latest session selector for CLI ask and chat"
```

**Step 3: Create linked PR**

Use the repository PR template, link `#759` with an explicit closing clause, and record the exact
verification commands and outcomes.
