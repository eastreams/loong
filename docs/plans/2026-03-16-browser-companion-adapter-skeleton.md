# Browser Companion Adapter Skeleton Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add the first LoongClaw-owned governed browser companion adapter surface so browser companion runtime readiness can unlock real discoverable tools instead of only a helper-skill preview path.

**Architecture:** Keep the shipped lightweight browser lane unchanged, then add a new `browser.companion.*` family behind the existing browser companion runtime gate. Read-oriented companion actions should execute as discoverable core tools with LoongClaw-owned session scope and typed protocol failures, while write-oriented companion actions should execute as discoverable app tools so the existing governed approval path can require review before execution.

**Tech Stack:** Rust, existing tool catalog/runtime config infrastructure, conversation turn engine, app-tool dispatcher, Markdown docs

---

### Task 1: Add failing browser companion tool-surface tests

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write failing catalog visibility tests**

Add tests that prove:

- `browser.companion.session.start`
- `browser.companion.navigate`
- `browser.companion.snapshot`
- `browser.companion.wait`
- `browser.companion.session.stop`
- `browser.companion.click`
- `browser.companion.type`

all stay hidden until `browser_companion.enabled=true` and `browser_companion.ready=true`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app browser_companion_visibility -- --nocapture`

Expected: FAIL because no governed browser companion tools exist yet.

**Step 3: Write failing tool discovery tests**

Add tests that prove:

- `tool.search` can find the companion tools once runtime-ready
- read-oriented actions resolve as core tools
- write-oriented actions resolve as app tools

**Step 4: Run test to verify it fails**

Run: `cargo test -p loongclaw-app browser_companion_tool_search -- --nocapture`

Expected: FAIL because catalog/search metadata does not yet include the companion family.

### Task 2: Add failing browser companion protocol tests

**Files:**
- Create: `crates/app/src/tools/browser_companion.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write failing protocol execution tests**

Add tests that prove:

- read-oriented tools fail closed when the companion runtime is disabled or not ready
- the configured companion command is invoked through a structured protocol boundary instead of raw shell text
- command spawn failures, non-zero exits, invalid JSON, and protocol-declared errors become typed adapter failures
- successful `session.start`, `navigate`, `snapshot`, `wait`, and `session.stop` responses preserve LoongClaw-issued session IDs and typed payload fields

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app browser_companion_protocol -- --nocapture`

Expected: FAIL because the browser companion adapter implementation does not exist yet.

**Step 3: Write minimal implementation**

- add a new browser companion module
- define a small request/response protocol around the configured companion command
- generate LoongClaw-owned companion session IDs and scope them per conversation session
- wire read-oriented tools through the core tool executor path

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app browser_companion_protocol -- --nocapture`

Expected: PASS

### Task 3: Add failing governed write-action tests

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/browser_companion.rs`

**Step 1: Write failing turn-engine approval tests**

Add tests that prove:

- `browser.companion.click` and `browser.companion.type` are discoverable app tools
- strict approval mode turns those write actions into persisted approval requests
- approved write actions execute through the app-tool dispatcher and preserve companion session scope
- read-oriented companion actions do not create governed approval requests under the same runtime state

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app browser_companion_approval -- --nocapture`

Expected: FAIL because the companion write actions are not yet routed through the governed app-tool path.

**Step 3: Write minimal implementation**

- assign approval-capable governance metadata to write-oriented companion actions
- extend app-tool dispatch to execute companion write requests
- keep read/write routing explicit and testable

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app browser_companion_approval -- --nocapture`

Expected: PASS

### Task 4: Sync product docs with the shipped partial adapter surface

**Files:**
- Modify: `docs/product-specs/browser-automation-companion.md`
- Modify: `docs/ROADMAP.md` if wording needs to distinguish adapter skeleton from full runtime

**Step 1: Update the product spec**

Document that LoongClaw now ships a partial governed adapter skeleton for the managed companion lane, while install/release lifecycle, isolated profile management, and broader runtime packaging remain planned work.

**Step 2: Verify docs mention the new scope truthfully**

Run: `rg -n "adapter skeleton|browser.companion|partial governed" docs/product-specs docs/ROADMAP.md`

Expected: PASS with the updated wording.

### Task 5: Verify, commit, and prepare PR delivery

**Files:**
- Modify only files in this task scope

**Step 1: Run focused validation**

Run:

```bash
cargo test -p loongclaw-app browser_companion_visibility -- --nocapture
cargo test -p loongclaw-app browser_companion_tool_search -- --nocapture
cargo test -p loongclaw-app browser_companion_protocol -- --nocapture
cargo test -p loongclaw-app browser_companion_approval -- --nocapture
```

Expected: PASS

**Step 2: Run repository validation**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
```

Expected: PASS

**Step 3: Commit cleanly**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Confirm only browser companion adapter skeleton files are staged, then commit with a focused message.
