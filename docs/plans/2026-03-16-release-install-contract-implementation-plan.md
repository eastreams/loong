# Release Install Contract Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close LoongClaw's release-first install contract with smoke coverage, actionable fallback guidance, and synced install docs.

**Architecture:** Keep the existing release-first installers, but make them smoke-testable through a local release-base override, improve missing-release guidance, and sync the public install docs to the current no-public-release reality.

**Tech Stack:** Bash, PowerShell, Markdown docs, existing shell test harnesses.

---

### Task 1: Update the design artifacts to the chosen slice

**Files:**
- Create: `docs/plans/2026-03-16-release-install-contract-design.md`
- Create: `docs/plans/2026-03-16-release-install-contract-implementation-plan.md`
- Delete: `docs/plans/2026-03-16-release-install-truthfulness-design.md`
- Delete: `docs/plans/2026-03-16-release-install-truthfulness-implementation-plan.md`

**Step 1: Rewrite the scope**

- Replace the earlier truthfulness-only framing with the chosen install-contract
  slice so the tracked design and implementation notes match the branch.

### Task 2: Add and verify the failing Bash smoke test

**Files:**
- Create: `scripts/test_install_sh.sh`
- Modify: `scripts/install.sh`

**Step 1: Keep the failing shell smoke coverage**

- Cover:
  - release-fixture install through `LOONGCLAW_INSTALL_RELEASE_BASE_URL`
  - checksum mismatch failure
  - missing latest-release guidance text

**Step 2: Run test to verify it fails**

Run: `bash scripts/test_install_sh.sh`
Expected: FAIL because `install.sh` does not yet honor the override or print the
full next-step guidance.

**Step 3: Implement minimal Bash installer changes**

- Add `LOONGCLAW_INSTALL_RELEASE_BASE_URL` support.
- Improve the no-release message with exact clone + source-install commands.
- Keep the rest of the release-first flow unchanged.

**Step 4: Run test to verify it passes**

Run: `bash scripts/test_install_sh.sh`
Expected: PASS

### Task 3: Mirror the behavior in PowerShell

**Files:**
- Modify: `scripts/install.ps1`

**Step 1: Keep PowerShell behavior parallel**

- Add the same `LOONGCLAW_INSTALL_RELEASE_BASE_URL` override.
- Improve the missing-release guidance with exact next actions.
- Preserve Windows-specific install behavior otherwise.

**Step 2: Verify as far as the environment allows**

Run: `command -v pwsh`
Expected: if unavailable, document that PowerShell parity was verified by
implementation review rather than local execution.

### Task 4: Sync the public install docs

**Files:**
- Modify: `README.md`
- Modify: `docs/product-specs/installation.md`

**Step 1: Update public install wording**

- Keep the release-first installer quickstart.
- Explicitly state that no public release is published yet.
- Point users to the source installer as the supported immediate fallback.
- Mark the installation spec acceptance criteria as shipped when the behavior is
  now real.

**Step 2: Re-run relevant checks**

Run: `bash scripts/test_install_sh.sh`
Expected: PASS

### Task 5: Full verification and GitHub delivery

**Files:**
- Modify: issue `#201` and open one PR against `alpha-test`

**Step 1: Format and validate**

Run: `cargo fmt --all -- --check`
Expected: PASS

**Step 2: Run targeted installer and release checks**

Run: `bash scripts/test_install_sh.sh`
Expected: PASS

Run: `bash scripts/test_release_artifact_lib.sh`
Expected: PASS

Run: `bash scripts/test_bootstrap_release_local_artifacts.sh`
Expected: PASS

**Step 3: Run Rust quality gates**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS

Run: `cargo test --workspace --all-features --locked`
Expected: PASS

**Step 4: Publish the GitHub artifacts**

- Update issue `#201` so its title/body match the install-contract scope.
- Commit only the install-contract slice.
- Push the branch to the operator fork.
- Open a PR against `alpha-test` with `Closes #201` in the body.
