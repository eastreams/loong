# Browser Companion Doctor Onboard Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Surface browser companion readiness and blockers through the existing
`loongclaw doctor` and `loongclaw onboard` flows without changing the current
tool visibility contract.

**Architecture:** Add one shared browser companion diagnostic snapshot that
turns config, command probe, version expectations, and runtime-ready flags into
typed health facts. Reuse that snapshot in daemon doctor checks, doctor next
steps, and onboarding preflight warnings so the same managed-lane truth reaches
both operator diagnostics and first-run guidance.

**Tech Stack:** Rust, Clap CLI, existing LoongClaw daemon/app config/runtime
helpers

---

### Task 1: Add failing browser companion doctor tests

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/tests/mod.rs`

**Step 1: Write failing tests for browser companion doctor checks**

Cover:
- enabled companion with no command configured
- enabled companion with missing binary command
- enabled companion with mismatched expected version
- enabled companion with matching command/version but runtime-ready flag absent

**Step 2: Run the focused tests and verify they fail**

Run:

```bash
cargo test -p loongclaw-daemon browser_companion_doctor -- --nocapture
```

Expected: FAIL because browser companion doctor checks do not exist yet.

### Task 2: Add failing onboarding preflight tests

**Files:**
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/tests/onboard_cli.rs`

**Step 1: Write failing onboarding tests**

Cover:
- preflight surfaces browser companion warnings when current config enables it
- non-interactive onboarding blocks on browser companion failures
- interactive preflight screen includes browser companion detail rows

**Step 2: Run the focused tests and verify they fail**

Run:

```bash
cargo test -p loongclaw-daemon browser_companion_onboard -- --nocapture
```

Expected: FAIL because onboarding does not yet reuse browser companion
diagnostics.

### Task 3: Implement shared browser companion diagnostics

**Files:**
- Create or modify under: `crates/daemon/src/doctor_cli.rs`
- Modify if needed: `crates/app/src/tools/runtime_config.rs`

**Step 1: Add a typed browser companion diagnostic snapshot**

The snapshot should capture:
- whether companion is requested by config
- effective command
- expected version
- command probe result
- detected version text
- runtime-ready flag state
- actionable failure reason

**Step 2: Keep the runtime visibility contract unchanged**

Do not change catalog gating or tool exposure behavior introduced by the
foundation branch. This feature is diagnostic-only.

### Task 4: Reuse diagnostics in doctor and onboarding

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/next_actions.rs` only if browser companion next
  actions need shared wording

**Step 1: Add doctor checks and next steps**

Doctor should emit browser companion checks only when the companion lane is
requested or explicitly configured, and should end with concrete next steps.

**Step 2: Add onboarding preflight integration**

Onboarding should reuse the same diagnostic facts instead of duplicating probe
logic.

**Step 3: Keep user language concrete**

Messages should tell operators exactly what to fix:
- configure command
- install/fix PATH
- align expected version
- rerun doctor

### Task 5: Verify and commit cleanly

Run:

```bash
cargo test -p loongclaw-daemon browser_companion_doctor -- --nocapture
cargo test -p loongclaw-daemon browser_companion_onboard -- --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
```

Expected result: PASS
