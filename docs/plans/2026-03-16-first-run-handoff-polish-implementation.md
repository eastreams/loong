# First-Run Handoff Polish Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Elevate the first runnable assistant handoff across onboarding, doctor, and chat so healthy first-run UX feels assistant-first instead of runtime-first.

**Architecture:** Keep the existing CLI/runtime surfaces intact, but adjust ordering and copy at the renderer boundary. Reuse `collect_setup_next_actions(...)` for shared action labels, move onboarding’s primary handoff above saved setup details, and restructure chat startup into assistant-first followed by compact detail sections.

**Tech Stack:** Rust, daemon/app unit and integration tests, Markdown docs/specs.

---

### Task 1: Lock the UX changes with failing tests

**Files:**
- Modify: `crates/daemon/tests/integration/onboard_cli.rs`
- Modify: `crates/app/src/chat.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/next_actions.rs`

**Step 1: Write failing onboarding summary tests**

- Update success-summary assertions so they expect:
  - the primary handoff to appear before saved setup inventory
  - the primary ask label to stop saying `ask example`

**Step 2: Write failing doctor copy tests**

- Tighten healthy-state next-step assertions around ask/chat copy.

**Step 3: Write failing chat startup tests**

- Update startup assertions so they expect:
  - a clearer first action
  - compact secondary headings for detail sections

**Step 4: Run targeted tests to verify RED**

Run:
- `cargo test -p loongclaw-app render_cli_chat_startup_lines -- --nocapture`
- `cargo test -p loongclaw-daemon onboarding_success_summary -- --nocapture`
- `cargo test -p loongclaw-daemon build_doctor_next_steps_promotes_ask_and_chat_when_green -- --nocapture`

Expected: FAIL because the current ordering/copy still reflects the old UX.

### Task 2: Implement the shared handoff polish

**Files:**
- Modify: `crates/daemon/src/next_actions.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/app/src/chat.rs`

**Step 1: Update shared ask labeling**

- Rename the shared ask action label to the product-facing wording chosen in
  the design.

**Step 2: Promote onboarding’s primary handoff**

- Reorder the success summary so:
  - the primary handoff appears immediately after the opening completion block
  - saved setup details are still preserved under a secondary heading

**Step 3: Tighten doctor copy**

- Keep doctor logic stable.
- Update the healthy-state ask/chat prefixes to the new product-shaped wording.

**Step 4: Reframe chat startup**

- Move startup copy into:
  - first action / usage hint
  - compact session details
  - compact runtime details

**Step 5: Run targeted tests to verify GREEN**

Run:
- `cargo test -p loongclaw-app render_cli_chat_startup_lines -- --nocapture`
- `cargo test -p loongclaw-daemon onboarding_success_summary -- --nocapture`
- `cargo test -p loongclaw-daemon build_doctor_next_steps_promotes_ask_and_chat_when_green -- --nocapture`

Expected: PASS

### Task 3: Sync docs and product specs

**Files:**
- Modify: `README.md`
- Modify: `docs/PRODUCT_SENSE.md`
- Modify: `docs/product-specs/onboarding.md`
- Modify: `docs/product-specs/doctor.md`
- Modify: `docs/product-specs/one-shot-ask.md`

**Step 1: Update docs**

- Describe the handoff-first first-run contract now shipped by `onboard`,
  `doctor`, and `chat`.

**Step 2: Run doc consistency checks**

Run:
- `rg -n "first answer|ask example|start here|Continue in chat|Get a first answer" README.md docs`

Expected: wording is aligned and `ask example` no longer appears in user-facing
docs/specs for the healthy first-run path.

### Task 4: Full verification and GitHub delivery

**Files:**
- Modify: one GitHub issue and one PR body

**Step 1: Full local verification**

Run:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Expected: PASS

**Step 2: GitHub delivery**

- Reuse the first-run handoff issue for this slice.
- Commit only this UX polish scope.
- Push to the operator fork.
- Open a PR against `alpha-test` with an explicit closing clause.

**Step 3: Workspace cleanup**

- Remove branch-local build artifacts before reporting completion.
