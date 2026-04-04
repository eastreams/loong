# CLI Process-Level Latest Selector Coverage Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add real spawned-process daemon integration coverage for `loong ask --session latest`
and `loong chat --session latest` without changing selector semantics.

**Architecture:** Keep the implementation test-only. Reuse sqlite-backed session seeding, extend
the existing spawned `chat` suite for REPL-visible behavior, and add a minimal spawned `ask` suite
that captures provider requests through a local mock server.

**Tech Stack:** Rust, tokio, axum or local TCP test server, rusqlite-backed session fixtures,
daemon integration tests.

---

## Task 1: Land the design and plan artifacts

**Files:**
- Create: `docs/plans/2026-04-01-cli-process-latest-selector-coverage-design.md`
- Create: `docs/plans/2026-04-01-cli-process-latest-selector-coverage-implementation-plan.md`

**Step 1: Write the artifacts**

- record that issue `#791` is specifically about process-level coverage beyond the already-landed
  app-layer tests
- keep the scope test-only and ownership-preserving

**Step 2: Verify artifacts exist**

Run:

```bash
test -f docs/plans/2026-04-01-cli-process-latest-selector-coverage-design.md
test -f docs/plans/2026-04-01-cli-process-latest-selector-coverage-implementation-plan.md
```

Expected: success

## Task 2: Capture the red baseline for relevant existing coverage

**Files:**
- Modify: none

**Step 1: Run the current latest-selector tests**

Run:

```bash
cargo test -p loongclaw-app cli_runtime_latest_session_selector --locked
cargo test -p loongclaw-daemon --test integration latest_session_selector --locked
```

Expected: green, confirming the current gap is specifically process-level coverage.

## Task 3: Add failing spawned chat coverage

**Files:**
- Modify: `crates/daemon/tests/integration/chat_cli.rs`

**Step 1: Add sqlite-backed chat fixture helpers**

- add minimal helpers that write a temp config file
- seed root, delegate-child, and archived sessions into sqlite memory
- keep helpers file-local unless duplication becomes clearly harmful

**Step 2: Add a failing happy-path test**

- spawn `loong chat --config <fixture> --session latest`
- pipe `/history 8` then `/exit`
- assert startup output and history reflect the latest resumable root session only

**Step 3: Add a failing no-match test**

- spawn `loong chat --config <fixture> --session latest`
- do not seed any eligible root session
- assert the process exits non-zero with a clear `latest` selector error

**Step 4: Run the focused chat tests and verify red**

Run:

```bash
cargo test -p loongclaw-daemon --test integration chat_cli_ --locked
```

Expected: fail because the new coverage does not exist yet or the fixture is incomplete.

## Task 4: Add failing spawned ask coverage

**Files:**
- Modify: `crates/daemon/tests/integration/mod.rs`
- Create: `crates/daemon/tests/integration/ask_cli.rs`

**Step 1: Register the new integration module**

- wire `ask_cli` into `crates/daemon/tests/integration/mod.rs`

**Step 2: Add a local mock provider helper**

- capture provider request bodies
- return a minimal successful completion payload
- avoid sleeps in the shared harness by waiting on recorded requests or immediate command
  completion
- if the old fixed wait budget needs an explicit regression proof, keep any setup delay bounded and
  local to that single regression test

**Step 3: Add a failing happy-path ask test**

- spawn `loong ask --config <fixture> --session latest --message ...`
- assert the captured provider request contains only the selected latest root history

**Step 4: Add a failing no-match ask test**

- run the same command without an eligible root session
- assert the process exits non-zero with a clear `latest` selector error
- assert no provider request was observed

**Step 5: Run the focused ask tests and verify red**

Run:

```bash
cargo test -p loongclaw-daemon --test integration ask_cli_ --locked
```

Expected: fail because the new coverage does not exist yet or the fixture is incomplete.

## Task 5: Implement the minimal fixture support needed for green

**Files:**
- Modify only the daemon integration test files touched above

**Step 1: Keep all support logic in test code unless a reusable helper is clearly justified**

- prefer small named helper functions
- keep session seeding explicit and readable
- avoid generic harness layers

**Step 2: Make the chat tests pass**

- ensure startup output and `/history` use the resolved session

**Step 3: Make the ask tests pass**

- ensure the mock provider captures request bodies deterministically
- ensure the no-match path exits before provider traffic

## Task 6: Run focused verification

**Files:**
- Modify: none unless validation exposes a necessary fix

**Run:**

```bash
cargo test -p loongclaw-daemon --test integration ask_cli_ --locked
cargo test -p loongclaw-daemon --test integration chat_cli_ --locked
cargo test -p loongclaw-daemon --test integration latest_session_selector --locked
```

Expected: green

## Task 7: Run broader verification

**Files:**
- Modify: none unless validation exposes a necessary fix

**Run:**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p loongclaw-app cli_runtime_latest_session_selector --locked
cargo test -p loongclaw-daemon --test integration --locked
cargo test --workspace --locked
cargo test --workspace --all-features --locked
git diff --check
```

Expected: all green, or any unrelated baseline failure must be separated explicitly before
claiming completion.

## Task 8: Prepare clean GitHub delivery

**Files:**
- Modify: GitHub artifacts through `gh`, not repository files

**Step 1: Inspect isolated changes**

Run:

```bash
git status --short
git diff -- docs/plans/2026-04-01-cli-process-latest-selector-coverage-design.md docs/plans/2026-04-01-cli-process-latest-selector-coverage-implementation-plan.md crates/daemon/tests/integration/mod.rs crates/daemon/tests/integration/chat_cli.rs crates/daemon/tests/integration/ask_cli.rs
```

**Step 2: Commit with a scoped message**

Run:

```bash
git add docs/plans/2026-04-01-cli-process-latest-selector-coverage-design.md docs/plans/2026-04-01-cli-process-latest-selector-coverage-implementation-plan.md crates/daemon/tests/integration/mod.rs crates/daemon/tests/integration/chat_cli.rs crates/daemon/tests/integration/ask_cli.rs
git commit -m "Add process-level latest selector CLI coverage"
```

**Step 3: Create a linked PR**

Use the repository PR template, link `#791` with an explicit closing clause, and record the exact
verification commands and outcomes.
