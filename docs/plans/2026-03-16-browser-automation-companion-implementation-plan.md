# Browser Automation Companion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a managed browser automation companion lane to LoongClaw without replacing the shipped lightweight browser tools or increasing default install friction.

**Architecture:** Keep `browser.open` / `browser.extract` / `browser.click` as the default safe browser lane, then layer an optional managed browser automation companion behind a LoongClaw-owned governed adapter, truthful tool visibility, and install/onboard/doctor health integration.

**Tech Stack:** Rust, existing tool/runtime policy infrastructure, install scripts, Markdown docs, optional companion packaging, GitHub release artifacts.

---

### Task 1: Land the design, spec, and roadmap contract

**Files:**
- Create: `docs/plans/2026-03-16-browser-automation-companion-design.md`
- Create: `docs/plans/2026-03-16-browser-automation-companion-implementation-plan.md`
- Create: `docs/product-specs/browser-automation-companion.md`
- Modify: `docs/product-specs/index.md`
- Modify: `docs/product-specs/browser-automation.md`
- Modify: `docs/ROADMAP.md`

**Step 1: Write the artifacts**

- record the two-lane browser model: shipped safe browser lane plus optional
  automation companion lane
- keep the companion lane explicitly marked as planned or expectation-setting,
  not already shipped
- define the role of a helper skill as additive guidance, not the runtime
  capability source of truth

**Step 2: Verify the artifacts exist**

Run: `test -f docs/plans/2026-03-16-browser-automation-companion-design.md && test -f docs/plans/2026-03-16-browser-automation-companion-implementation-plan.md && test -f docs/product-specs/browser-automation-companion.md`

Expected: success

### Task 2: Add companion runtime configuration and readiness model

**Files:**
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- a distinct browser companion config surface exists
- companion tools remain hidden when the runtime is disabled or unhealthy
- runtime-visible tool catalogs and provider definitions expose the same
  companion-visible set
- the shipped lightweight browser tools remain unaffected when the companion is
  off

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app tool_visibility companion`

Expected: FAIL because the runtime config and tool visibility model do not yet
understand a companion lane.

**Step 3: Write minimal implementation**

- add companion enablement and readiness inputs to runtime config
- extend tool descriptors with companion-lane visibility support
- keep base browser tools and companion tools separately testable

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tool_visibility companion`

Expected: PASS

### Task 3: Add doctor and onboarding companion health checks

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/daemon/src/tests/doctor_cli.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- `onboard` can present enhanced browser automation as an optional companion
  choice
- `doctor` reports missing companion runtime, version mismatch, or missing
  isolated profile as next-action guidance
- healthy companion state is rendered distinctly from the default safe browser
  lane

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon onboard doctor companion`

Expected: FAIL because onboarding and doctor do not yet understand the companion
  readiness model.

**Step 3: Write minimal implementation**

- add companion-aware onboarding prompts and summaries
- add doctor checks for runtime presence, version compatibility, and profile
  health
- render next-step actions instead of raw missing-state text

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon onboard doctor companion`

Expected: PASS

### Task 4: Add companion install and packaging hooks

**Files:**
- Modify: `scripts/install.sh`
- Modify: `scripts/install.ps1`
- Modify: `.github/workflows/release.yml`
- Modify: `scripts/test_release_artifact_lib.sh`

**Step 1: Write the failing tests**

Add tests that prove:

- the install flow can optionally request the browser automation companion pack
- unsupported platforms fail with a concrete next step
- the release workflow publishes the companion artifact or manifest expected by
  the installers

**Step 2: Run test to verify it fails**

Run: `bash scripts/test_release_artifact_lib.sh`

Expected: FAIL because install and release artifacts do not yet expose a
  companion pack contract.

**Step 3: Write minimal implementation**

- define artifact naming or manifest format for the companion lane
- add install flags or onboarding-assisted install steps
- publish the required release metadata

**Step 4: Run test to verify it passes**

Run: `bash scripts/test_release_artifact_lib.sh`

Expected: PASS

### Task 5: Add the governed browser automation adapter

**Files:**
- Create: `crates/app/src/tools/browser_companion.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Test: `crates/app/src/tools/browser_companion.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- companion sessions can be started only when the runtime is ready
- navigation, click, type, wait, extract, screenshot, and session stop return
  typed outcomes
- read actions and write actions follow different policy or approval paths
- session IDs remain scoped and auditable

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app browser_companion`

Expected: FAIL because the governed adapter does not yet exist.

**Step 3: Write minimal implementation**

- implement the companion adapter boundary
- map companion operations into LoongClaw-owned tool payloads and results
- emit audit events and typed failures

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app browser_companion`

Expected: PASS

### Task 6: Add profile isolation and approval evidence

**Files:**
- Modify: `crates/app/src/tools/approval.rs`
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/context.rs`
- Test: `crates/app/src/tools/approval.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- companion profiles are isolated from unrelated LoongClaw sessions
- write-oriented page actions request stronger authorization than read actions
- profile lifecycle and approval decisions are traceable in audit evidence

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app approval companion profile`

Expected: FAIL because profile isolation and approval evidence are not yet wired
  for the companion lane.

**Step 3: Write minimal implementation**

- add isolated profile identifiers and runtime state plumbing
- bind high-risk page actions to stronger approval handling
- emit structured audit fields that explain what the browser session attempted

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app approval companion profile`

Expected: PASS

### Task 7: Add helper skill and example flows

**Files:**
- Create or Modify: packaged helper skill files under the selected first-party
  skill location
- Modify: `README.md`
- Modify: `docs/product-specs/browser-automation-companion.md`

**Step 1: Write the failing docs or packaging checks**

- define how the helper skill is bundled, discovered, or recommended
- make sure docs do not imply the helper skill alone provides the runtime

**Step 2: Run the checks**

Run: `rg -n "helper skill|companion|browser.session.start|browser.screenshot" README.md docs`

Expected: matches only where the new product story is defined correctly.

**Step 3: Write minimal implementation**

- add first-party helper skill content with task recipes
- keep examples aligned with actual companion availability rules

**Step 4: Re-run the checks**

Run: `rg -n "helper skill|companion|browser.session.start|browser.screenshot" README.md docs`

Expected: PASS with consistent language.

### Task 8: Full verification and delivery

**Files:**
- Modify only what the previous tasks require

**Step 1: Run focused verification**

Run: `bash scripts/test_release_artifact_lib.sh`
Run: `cargo test -p loongclaw-daemon onboard doctor companion`
Run: `cargo test -p loongclaw-app browser_companion tool_visibility approval`

Expected: PASS

**Step 2: Run broad verification**

Run: `cargo fmt --all -- --check`
Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Run: `cargo test --workspace --all-features --locked`

Expected: PASS

**Step 3: Commit**

```bash
git add docs/plans docs/product-specs docs/ROADMAP.md README.md scripts .github/workflows crates/app crates/daemon
git commit -m "feat(browser): define managed automation companion lane"
```

**Step 4: Push and open PR**

- push branch to the fork remote
- open PR against `alpha-test`
- link the tracking issue with an explicit closing clause
