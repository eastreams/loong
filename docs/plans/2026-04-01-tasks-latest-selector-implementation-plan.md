# Tasks CLI Latest Session Selector Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `--session latest` support to the `loongclaw tasks` command family without changing
literal session handling or widening selector semantics beyond this entry point.

**Architecture:** Keep the selector resolver local to `crates/daemon/src/tasks_cli.rs`, and reuse
`SessionRepository::latest_resumable_root_session_summary()` as the source of truth for the newest
resumable root session.

**Tech Stack:** Rust, tokio, clap, rusqlite-backed SQLite test fixtures, daemon integration tests

---

### Task 1: Write the failing tasks selector tests

**Files:**
- Modify: `crates/daemon/tests/integration/tasks_cli.rs`

**Step 1: Add the fixture helpers needed for selector ordering**

Add small test helpers that can:

- append a turn to a root session
- set session `updated_at`
- set turn timestamps

Keep the helpers local to `tasks_cli.rs`.

**Step 2: Write the failing success-path test**

Create:

- one older resumable root session
- one newer resumable root session

Run `execute_tasks_command()` with `TasksCommands::Create` and `session: "latest"`.

Assert:

- `current_session_id` resolves to the newer root session
- the created delegate child session is parented to that newer root session

**Step 3: Write the failing no-match test**

Run `execute_tasks_command()` with `session: "latest"` and no resumable root session in the test
fixture.

Assert:

- the command returns an error
- the error mentions `latest`

**Step 4: Run the focused tasks tests and verify red**

Run:

```bash
cargo test -p loongclaw --test integration latest_session_selector --locked
```

Expected: the new tests fail because `tasks` still treats `latest` as a literal session id.

### Task 2: Implement selector-aware tasks session resolution

**Files:**
- Modify: `crates/daemon/src/tasks_cli.rs`

**Step 1: Keep the existing trim and empty validation**

Retain the current empty-string rejection path for `tasks`.

**Step 2: Add a thin `latest` resolver**

Add a local helper that:

- receives the trimmed session value
- returns literal values unchanged
- resolves `latest` through `SessionRepository::latest_resumable_root_session_summary()`
- returns a clear no-match error if the repository has no resumable root session

**Step 3: Wire the resolver into `execute_tasks_command()`**

Resolve the session after `MemoryRuntimeConfig` is available and before subcommand dispatch.

**Step 4: Run the focused tasks tests and verify green**

Run:

```bash
cargo test -p loongclaw --test integration latest_session_selector --locked
```

Expected: the selector tests pass.

### Task 3: Re-run the existing tasks integration surface

**Files:**
- Modify: none unless a regression is exposed

**Step 1: Run the broader tasks integration coverage**

Run:

```bash
cargo test -p loongclaw --test integration tasks_ --locked
```

Expected: existing literal-session command coverage stays green.

**Step 2: Keep scope tight**

Do not change CLI parsing, repository semantics, or unrelated command families unless the tests
prove a real regression or wiring gap.

### Task 4: Run full verification for delivery

**Files:**
- Modify: none unless verification exposes a necessary fix

**Step 1: Run formatting and diff hygiene**

Run:

```bash
cargo fmt --all -- --check
git diff --check
```

**Step 2: Run workspace-level static checks**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

**Step 3: Run broad workspace tests**

Run:

```bash
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

Record any unrelated baseline issue explicitly instead of folding it into this change.

### Task 5: Prepare clean GitHub delivery

**Files:**
- Modify: GitHub artifacts through `gh`, not repository files

**Step 1: Inspect the isolated diff**

Run:

```bash
git status --short
git diff -- crates/daemon/src/tasks_cli.rs crates/daemon/tests/integration/tasks_cli.rs docs/plans/2026-04-01-tasks-latest-selector-design.md docs/plans/2026-04-01-tasks-latest-selector-implementation-plan.md
```

**Step 2: Commit with a scoped message**

Run:

```bash
git add crates/daemon/src/tasks_cli.rs crates/daemon/tests/integration/tasks_cli.rs docs/plans/2026-04-01-tasks-latest-selector-design.md docs/plans/2026-04-01-tasks-latest-selector-implementation-plan.md
git commit -m "Add latest session selector support for tasks CLI"
```

**Step 3: Open the PR with the repository template**

Create a PR that:

- links `#800`
- uses `Closes #800`
- records the exact verification commands and outcomes
- keeps the scope boundary explicit: `tasks` only
