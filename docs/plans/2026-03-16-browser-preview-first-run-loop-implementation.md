# Browser Preview First-Run Loop Implementation Plan

> **Required execution note:** follow this plan task-by-task using the available execution tooling.

**Goal:** Productize the existing browser preview path so users can enable it, diagnose missing runtime state, and try concrete first recipes directly from LoongClaw CLI surfaces.

**Architecture:** Extend the shared browser-preview guidance in `crates/daemon/src/browser_preview.rs`, route `next_actions` and `doctor` through that richer guidance, and upgrade `skills enable-browser-preview` text mode to surface install steps and recipes without changing the underlying preview runtime model.

**Tech Stack:** Rust, existing daemon CLI integration tests, Markdown docs/specs.

---

## Task 1: Lock the slice with failing tests

**Files:**
- Modify: `crates/daemon/src/next_actions.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/tests/integration/onboard_cli.rs`
- Modify: `crates/daemon/tests/integration/skills_cli.rs`

**Step 1: Write failing next-action tests**

- Add or tighten assertions so:
  - browser preview ready state expects a task-shaped recipe command
  - browser preview install-runtime state expects a real install action, not
    `agent-browser open example.com`

**Step 2: Write failing doctor guidance tests**

- Add assertions that doctor next steps include:
  - an exact `agent-browser` install command
  - a verify command
  - the browser preview action still visible beside ask/chat when relevant

**Step 3: Write failing skills CLI text test**

- Add a text-rendering regression test for `skills.enable-browser-preview`
  output that expects:
  - config/runtime summary
  - next-step install or verify guidance
  - 2 to 3 recipe commands

**Step 4: Run failing tests**

Run:
- `cargo test -p loongclaw-daemon next_actions -- --nocapture`
- `cargo test -p loongclaw-daemon doctor -- --nocapture`
- `cargo test -p loongclaw-daemon skills_cli -- --nocapture`

Expected: FAIL because the current guidance is still too technical.

## Task 2: Implement shared browser preview guidance

**Files:**
- Modify: `crates/daemon/src/browser_preview.rs`
- Modify: `crates/daemon/src/next_actions.rs`

**Step 1: Add shared install and verify commands**

- Introduce shared helpers for:
  - recommended `agent-browser` install command
  - verify command

**Step 2: Add shared recipes**

- Add 2 to 3 browser preview recipe definitions and a helper that formats them
  into `loongclaw ask --config ... --message "..."` commands.

**Step 3: Update next-actions behavior**

- Ready state returns the first shared recipe command.
- Install-runtime state returns the shared install action and a clearer label.

**Step 4: Run targeted tests**

Run:
- `cargo test -p loongclaw-daemon next_actions -- --nocapture`

Expected: PASS

## Task 3: Upgrade doctor and skills CLI surfaces

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/skills_cli.rs`
- Modify: `crates/daemon/tests/integration/skills_cli.rs`

**Step 1: Improve doctor next steps**

- Replace the raw missing-runtime message with:
  - install command
  - verify command
- Keep ask/chat/browser ordering stable.

**Step 2: Improve `skills enable-browser-preview` text mode**

- Keep JSON behavior compatible.
- Add product-style text output showing:
  - preview enabled summary
  - runtime readiness
  - next actions
  - 2 to 3 recipe commands

**Step 3: Run targeted tests**

Run:
- `cargo test -p loongclaw-daemon doctor -- --nocapture`
- `cargo test -p loongclaw-daemon skills_cli -- --nocapture`

Expected: PASS

## Task 4: Sync docs and specs

**Files:**
- Modify: `README.md`
- Modify: `docs/product-specs/onboarding.md`
- Modify: `docs/product-specs/doctor.md`
- Modify: `docs/product-specs/browser-automation-companion.md`

**Step 1: Update browser preview README copy**

- Document the one-command enable path, the runtime install truth, and the new
  recipes.

**Step 2: Update product specs**

- Mark shipped acceptance items that now match behavior.
- Add any precise wording needed for runtime install hints and first-task
  recipes.

**Step 3: Re-run doc spot checks**

Run:
- `rg -n "enable-browser-preview|agent-browser|browser companion preview|ask --config" README.md docs/product-specs`

Expected: updated wording is consistent.

## Task 5: Verify, publish, and clean up

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

- Reuse an existing issue if a narrow one already matches; otherwise open a new
  issue linked back to umbrella `#168`.
- Commit only this browser-preview first-run loop slice.
- Push to the operator fork.
- Open a PR against `alpha-test` with an explicit closing clause.

**Step 3: Workspace cleanup**

- Remove branch-local `target/` before reporting completion.
