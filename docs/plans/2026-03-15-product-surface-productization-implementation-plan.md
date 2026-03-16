# Product Surface Productization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Productize the next LoongClaw MVP slice by making release-backed install the default path, adding minimal safe browser automation, polishing first-run assistant outputs, and unifying tool visibility truthfulness.

**Architecture:** Reuse the current release workflow and install scripts for distribution, extend the existing tool catalog/runtime plane with a lightweight HTML browser session layer, and route product-facing capability copy through the same runtime-visible tool view that governs actual execution.

**Tech Stack:** Rust, Clap, serde, reqwest, shell scripts, GitHub Releases, Markdown docs.

---

## Task 1: Land the design and issue linkage

**Files:**
- Create: `docs/plans/2026-03-15-product-surface-productization-design.md`
- Create: `docs/plans/2026-03-15-product-surface-productization-implementation-plan.md`

**Step 1: Write the artifacts**

- record the LoongClaw-native decision to prefer release-first install and a lightweight HTML
  browser over full browser parity
- keep the scope explicitly tied to issue `#168`

**Step 2: Verify artifacts exist**

Run: `test -f docs/plans/2026-03-15-product-surface-productization-design.md && test -f docs/plans/2026-03-15-product-surface-productization-implementation-plan.md`

Expected: success

### Task 2: Add failing installer tests for release-backed distribution

**Files:**
- Modify: `scripts/test_release_artifact_lib.sh`
- Modify: `scripts/install.sh`
- Modify: `scripts/install.ps1`
- Modify: `.github/workflows/release.yml`

**Step 1: Write the failing tests**

Add shell-level tests that prove:

- platform/architecture resolution maps to the release artifact names emitted by the workflow
- installers prefer release downloads by default
- source-build fallback is explicit instead of implicit
- checksum metadata is available for verification

**Step 2: Run test to verify it fails**

Run: `bash scripts/test_release_artifact_lib.sh`

Expected: FAIL because the installer/release helpers do not yet expose the new release-download
path or checksum contract.

**Step 3: Write minimal implementation**

- add shared release asset naming/checksum helpers
- update the release workflow to publish checksum files alongside archives
- teach `install.sh` / `install.ps1` to download and verify the correct release asset for the host
  platform

**Step 4: Run test to verify it passes**

Run: `bash scripts/test_release_artifact_lib.sh`

Expected: PASS

### Task 3: Add browser config and visibility tests

**Files:**
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- a new browser policy/config surface exists
- runtime tool view advertises browser tools only when they are enabled
- provider definitions and capability snapshot expose the same browser-visible set
- disabled browser policy removes the tools from all product-facing surfaces

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app tools:: runtime_config::tests:: tool_config_defaults_are_safe`

Expected: FAIL because the browser policy and tool descriptors do not exist yet.

**Step 3: Write minimal implementation**

- add browser config/runtime-policy types
- register browser descriptors
- route capability snapshot and provider definitions through the same runtime-visible tool view

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools:: runtime_config::tests::`

Expected: PASS

### Task 4: Add failing browser execution tests

**Files:**
- Create: `crates/app/src/tools/browser.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/browser.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- `browser.open` rejects disabled config and unsafe/private hosts by default
- `browser.open` stores a page session with readable metadata
- `browser.extract` can return page text, title, and visible links from the current page
- `browser.click` only follows safe link targets discovered from the current page
- browser session state remains bounded and deterministic

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app browser`

Expected: FAIL because the browser tool executor does not exist yet.

**Step 3: Write minimal implementation**

- implement a lightweight HTML browser session executor
- reuse existing web policy validation and limits wherever possible
- keep session state in a bounded process-local store for the running daemon/app process

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app browser`

Expected: PASS

### Task 5: Add failing first-run UX tests

**Files:**
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/app/src/chat.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- onboarding success recommends a concrete `ask --message ...` handoff
- doctor prints recommended next actions for common warning/failure states
- ask prints a concise productized header before the assistant answer
- chat startup copy becomes shorter and more assistant-oriented while preserving help commands

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon onboard`  
Run: `cargo test -p loongclaw-app chat`

Expected: FAIL because the current copy does not expose the new productized guidance.

**Step 3: Write minimal implementation**

- adjust onboarding next-action selection and copy
- add doctor next-action derivation and rendering
- add compact ask/chat presentation helpers

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon onboard doctor`  
Run: `cargo test -p loongclaw-app chat`

Expected: PASS

### Task 6: Update docs for the shipped product path

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/ROADMAP.md`
- Modify: `docs/product-specs/onboarding.md`
- Modify: `docs/product-specs/doctor.md`
- Modify: `docs/product-specs/one-shot-ask.md`

**Step 1: Update docs after behavior is green**

- move prebuilt install to the top of quick start
- describe the lightweight browser capability accurately
- keep first-run path aligned with shipped `onboard -> ask/chat -> doctor` guidance

**Step 2: Verify docs reference the new surfaces**

Run: `rg -n "browser.open|browser.extract|browser.click|prebuilt|GitHub Release|ask --message" README.md README.zh-CN.md docs`

Expected: matches for the new shipped path

### Task 7: Full verification and delivery

**Files:**
- Modify only what the previous tasks require

**Step 1: Run focused verification**

Run: `bash scripts/test_release_artifact_lib.sh`  
Run: `cargo fmt --all -- --check`  
Run: `cargo test -p loongclaw-app browser tools:: runtime_config::tests::`  
Run: `cargo test -p loongclaw-daemon onboard doctor`

Expected: PASS

**Step 2: Run broader verification**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: PASS

**Step 3: Commit**

```bash
git add docs/plans README.md README.zh-CN.md .github/workflows/release.yml scripts crates/app crates/daemon docs/product-specs docs/ROADMAP.md
git commit -m "feat(product): productize install, browser, and first-run surfaces"
```

**Step 4: Push and open PR**

- push branch to fork remote
- open PR against `alpha-test`
- use PR body `Closes #168`
