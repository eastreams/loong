# Delegate Runtime Contract Visibility V1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Surface the effective delegate child runtime contract in child planning prompts without changing provider tool schema or runtime enforcement behavior.

**Architecture:** Format a deterministic prompt block directly from `ToolRuntimeNarrowing`, then inject it through the existing system-prompt addition path inside `DefaultConversationRuntime::build_context(...)` only for child sessions with non-empty persisted runtime narrowing.

**Tech Stack:** Rust, serde-backed runtime narrowing structs, conversation runtime prompt assembly, memory-backed session lifecycle loading, existing conversation/runtime and runtime-config tests.

---

### Task 1: Add failing tests for prompt contract visibility

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`

**Step 1: Add a failing child-session prompt test**

Add a `memory-sqlite` conversation test that persists a child session with delegate
`runtime_narrowing`, calls `DefaultConversationRuntime::build_context(...)`, and asserts the system
prompt contains the delegate runtime contract marker plus narrowed web/browser values.

**Step 2: Add a failing root-session negative control**

Add a conversation test proving a root session prompt does not include the delegate runtime contract
marker.

**Step 3: Add a failing empty-narrowing negative control**

Add a child-session conversation test where the persisted runtime narrowing is empty and verify no
contract block is injected.

**Step 4: Add a failing formatter test**

Add a `ToolRuntimeNarrowing` unit test proving the formatted prompt block is deterministic and only
contains configured narrowing fields.

**Step 5: Run the focused red tests**

Run only the new conversation/runtime and runtime-config tests and confirm they fail for the missing
prompt-summary behavior before implementing production code.

### Task 2: Implement the prompt-summary formatter

**Files:**
- Modify: `crates/app/src/tools/runtime_config.rs`

**Step 1: Add a stable formatter helper**

Add a method on `ToolRuntimeNarrowing` that returns `None` for empty narrowing and otherwise returns
a deterministic prompt block string.

**Step 2: Keep formatting aligned with enforcement semantics**

Emit only fields that are actually narrowed:

- `web.fetch allow_private_hosts`
- `web.fetch allowed_domains`
- `web.fetch blocked_domains`
- `web.fetch timeout_seconds`
- `web.fetch max_bytes`
- `web.fetch max_redirects`
- `browser max_sessions`
- `browser max_links`
- `browser max_text_chars`

**Step 3: Preserve deterministic output**

Use fixed line ordering and sorted set iteration so tests and prompt behavior stay stable.

### Task 3: Inject the child contract through existing prompt rewrite logic

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`

**Step 1: Add a helper to derive the child prompt addition**

Create a small helper that reads `SessionContext.runtime_narrowing` and returns the formatted prompt
summary only for child sessions.

**Step 2: Merge additions instead of replacing them**

Combine any context-engine `system_prompt_addition` with the delegate runtime contract block so the
existing addition behavior remains intact.

**Step 3: Reuse `apply_system_prompt_addition(...)`**

Keep the system prompt injection path unchanged apart from feeding it the merged addition text.

### Task 4: Verify locally and prepare GitHub delivery

**Files:**
- Modify: GitHub issue / PR artifacts after code lands

**Step 1: Run focused tests**

Run the newly added conversation/runtime and runtime-config tests.

**Step 2: Run repository verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

**Step 3: Prepare stacked delivery**

Commit only this slice, push `feat/delegate-runtime-contract-visibility-v1`, and open a stacked PR
against the current delegate-runtime branch with `Closes #282` in the PR body.
