# Feishu Media Transfer Retry Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend Feishu-only retry hardening so media uploads/downloads remain stable under transient Feishu rate limiting and retryable failures.

**Architecture:** Keep retry semantics inside `FeishuClient`, using targeted request rebuild for multipart uploads and shared retry classification for JSON and binary paths. Limit resource-layer changes to constructing replayable multipart forms and keep all behavior scoped to `app/src/feishu/*`.

**Tech Stack:** Rust, reqwest, axum test server, tokio, serde_json, chrono

---

### Task 1: Add failing multipart and Retry-After coverage

**Files:**
- Modify: `crates/app/src/feishu/client.rs`

**Step 1: Write the failing test**

Add focused regressions for:
- multipart upload retries after a retryable Feishu payload error
- binary download retries after a `429` response and preserves metadata
- `Retry-After` accepts HTTP-date and saturates past dates to zero delay

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app --target-dir target/codex-feishu-media-retry post_multipart_retries_ -- --nocapture --test-threads=1`

Expected: FAIL because multipart retries cannot yet rebuild the request or HTTP-date parsing is incomplete.

**Step 3: Write minimal implementation**

Refactor Feishu client retry helpers so multipart attempts can rebuild their `Form` payload per retry and extend `Retry-After` parsing to support HTTP-date.

**Step 4: Run test to verify it passes**

Run the new focused tests again and confirm they pass.

**Step 5: Commit**

Skip commit unless explicitly requested.

### Task 2: Align media resource call sites with replayable multipart construction

**Files:**
- Modify: `crates/app/src/feishu/resources/media.rs`
- Test: `crates/app/src/feishu/resources/media.rs`

**Step 1: Write the failing test**

Add or adjust resource-level regression coverage only if client signature changes require replayable form construction.

**Step 2: Run test to verify it fails**

Run the resource-specific test if added.

**Step 3: Write minimal implementation**

Update upload helpers to provide replayable multipart construction while keeping file/media logic in `resources/media.rs`.

**Step 4: Run test to verify it passes**

Confirm focused resource/media tests pass.

**Step 5: Commit**

Skip commit unless explicitly requested.

### Task 3: Full verification

**Files:**
- Modify: none expected

**Step 1: Run format verification**

Run: `cargo fmt --all`

Expected: exit 0

**Step 2: Run application verification**

Run: `cargo test -p loongclaw-app --target-dir target/codex-feishu-media-retry -- --nocapture --test-threads=1`

Expected: PASS

**Step 3: Run daemon verification**

Run: `cargo test -p loongclaw-daemon --target-dir target/codex-feishu-media-retry -- --nocapture --test-threads=1`

Expected: PASS

**Step 4: Run diff health check**

Run: `git diff --check`

Expected: no output

**Step 5: Commit**

Skip commit unless explicitly requested.
