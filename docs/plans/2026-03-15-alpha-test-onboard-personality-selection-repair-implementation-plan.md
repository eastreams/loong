# Alpha-Test Onboard Personality Selection Repair Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Repair `alpha-test` onboarding so personality selection, memory-profile selection, and inline prompt override semantics are correct again across runtime, tests, and docs.

**Architecture:** Keep the current unified onboarding/import flow, but reintroduce personality and memory-profile handling as first-class onboarding fields. Align CLI-domain diff/planning logic with prompt-pack metadata so review/import behavior matches runtime semantics instead of only comparing `system_prompt`.

**Tech Stack:** Rust, clap, existing `loongclaw-app` config/prompt/runtime modules, daemon onboarding/import flow, Markdown docs, `cargo test`, `cargo fmt`, `cargo clippy`.

---

### Task 1: Add failing CLI parser coverage for the missing flags

**Files:**
- Modify: `crates/daemon/src/main.rs`

**Step 1: Write the failing test**

Add parser tests that expect `loongclaw onboard --personality friendly_collab`
and `loongclaw onboard --memory-profile profile_plus_window` to parse into the
`Onboard` command.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon onboard_cli_accepts_personality_flag -- --exact`
Expected: FAIL because the current clap command has no `personality` field.

**Step 3: Write minimal implementation**

Add the clap fields and thread them into `OnboardCommandOptions`.

**Step 4: Run test to verify it passes**

Run the same targeted test and the matching memory-profile parser test.

**Step 5: Commit**

Commit message: `test: restore onboard parser coverage for personality and memory profile`

### Task 2: Add failing onboarding behavior tests for detailed guided flow

**Files:**
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/onboard_presentation.rs`

**Step 1: Write the failing test**

Add transcript-level tests that assert the guided flow includes personality and
memory-profile steps, and that the resulting config uses the selected values.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon onboard_guided_flow_captures_personality_and_memory_profile -- --exact`
Expected: FAIL because the current guided flow only has `system prompt`.

**Step 3: Write minimal implementation**

Reintroduce guided personality and memory-profile selection inside the unified
flow without removing current review/shortcut behavior.

**Step 4: Run test to verify it passes**

Run the targeted guided-flow test and nearby onboarding transcript tests.

**Step 5: Commit**

Commit message: `feat: restore guided onboard personality and memory profile selection`

### Task 3: Add failing regression tests for inline prompt override semantics

**Files:**
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`

**Step 1: Write the failing test**

Add onboarding tests that prove explicit inline `system_prompt` selection clears
`prompt_pack_id`, `personality`, and `system_prompt_addendum`, and that the
resolved runtime system message starts with the inline prompt.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon non_interactive_system_prompt_override_disables_prompt_pack -- --exact`
Expected: FAIL because onboarding currently only mutates `cli.system_prompt`.

**Step 3: Write minimal implementation**

Update onboarding selection logic so full inline override explicitly disables
native prompt-pack metadata.

**Step 4: Run test to verify it passes**

Run the targeted onboarding test and the relevant app/provider prompt test.

**Step 5: Commit**

Commit message: `fix: restore truthful onboard inline prompt override semantics`

### Task 4: Add failing CLI-domain migration tests for prompt metadata

**Files:**
- Modify: `crates/daemon/src/migration/discovery.rs`
- Modify: `crates/daemon/src/migration/planner.rs`
- Modify: `crates/daemon/src/tests/onboard_cli.rs`

**Step 1: Write the failing test**

Add tests showing that prompt-pack metadata and addendum differences are
detected as CLI-domain differences and preserved by supplementation logic.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon cli_import_surface_detects_prompt_pack_metadata_changes -- --exact`
Expected: FAIL because current discovery/planner code only compares
`system_prompt` and `exit_commands`.

**Step 3: Write minimal implementation**

Expand CLI-domain comparison and supplementation logic to account for prompt
pack id, personality, and addendum semantics.

**Step 4: Run test to verify it passes**

Run the new targeted tests plus nearby discovery/planner tests.

**Step 5: Commit**

Commit message: `fix: align onboard migration cli domain with prompt metadata`

### Task 5: Add docs coverage and README parity

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/plans/2026-03-15-alpha-test-onboard-personality-selection-repair-design.md`

**Step 1: Write the failing check**

Use the repaired behavior as the source of truth and identify any README claims
that remain out of sync after code changes.

**Step 2: Verify the mismatch exists**

Run targeted text inspection with `rg`/`sed`.
Expected: current Chinese README lacks personality onboarding content.

**Step 3: Write minimal implementation**

Update both READMEs so they describe the same restored onboarding behavior and
do not overclaim beyond the implemented flow.

**Step 4: Verify the docs match**

Re-read the affected sections and confirm terminology parity.

**Step 5: Commit**

Commit message: `docs: align onboard personality and memory profile docs`

### Task 6: Full validation and GitHub delivery

**Files:**
- Modify: `.github/PULL_REQUEST_TEMPLATE.md` only if repository policy requires template updates
- Add or update: GitHub issue and PR artifacts

**Step 1: Run focused validation**

Run targeted `cargo test` commands for each repaired regression cluster.

**Step 2: Run repository validation**

Run:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`

If the shared cargo queue blocks progress, wait and rerun instead of claiming
success without evidence.

**Step 3: Review git isolation**

Run:
- `git status --short`
- `git diff --cached --name-only`
- `git diff --cached`

Ensure only task-scoped changes are included.

**Step 4: Publish branch and GitHub artifacts**

- Open or update the issue using the repository bug template fields.
- Push the branch to `origin`.
- Open a PR against `upstream/alpha-test` using the PR template and `Closes #<id>`.

**Step 5: Final verification**

Re-check local validation output, PR body, linked issue, and branch status.

**Step 6: Commit**

Commit message: `fix: restore onboard personality and memory profile flows`
