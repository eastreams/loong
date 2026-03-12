# User-Facing Setup Removal Polish Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the last user-visible `setup` wording that can imply the deleted CLI subcommand still exists.

**Architecture:** Keep the already-merged command removal intact and tighten only the user-facing surfaces that remain live today. Cover the CLI help boundary with a regression test, then update active onboarding/help/docs strings to consistently describe onboarding and diagnostics.

**Tech Stack:** Rust, Clap, cargo test, Markdown docs

---

### Task 1: Lock CLI help wording

**Files:**
- Modify: `crates/daemon/src/main.rs`

**Step 1: Write the failing test**

Add a CLI regression test that renders `Cli::command()` help output and asserts:
- help contains `onboarding`
- help does not contain `setup`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon --manifest-path Cargo.toml cli_tests::root_help_uses_onboarding_language`
Expected: FAIL because current help text still contains `setup`.

**Step 3: Write minimal implementation**

Update the `Onboard` and `Doctor` subcommand descriptions so current help no longer uses `setup`
wording that points users toward the removed command.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon --manifest-path Cargo.toml cli_tests::root_help_uses_onboarding_language`
Expected: PASS.

### Task 2: Clean onboarding and active docs

**Files:**
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`

**Step 1: Write the failing test**

Add a focused assertion in onboarding tests for the non-interactive model-probe failure message, or
extract the string behind a helper and test that helper directly.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon --manifest-path Cargo.toml onboard`
Expected: FAIL because the current retry hint still says `during setup`.

**Step 3: Write minimal implementation**

Replace user-facing wording with `onboarding` / `first-run` phrasing in the onboarding retry hint
and in active README text where the legacy noun still appears.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon --manifest-path Cargo.toml onboard`
Expected: PASS.

### Task 3: Verify and finalize

**Files:**
- Modify: none expected

**Step 1: Run format check**

Run: `cargo fmt --all --manifest-path Cargo.toml -- --check`
Expected: PASS.

**Step 2: Run lint**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS.

**Step 3: Run relevant package tests**

Run:
- `cargo test -p loongclaw-daemon --manifest-path Cargo.toml`
- `cargo test -p loongclaw-app --manifest-path Cargo.toml`

Expected: PASS.

**Step 4: Inspect diff and commit**

Run:
- `git status --short`
- `git diff --stat`

Then create a focused commit for the user-facing polish.
