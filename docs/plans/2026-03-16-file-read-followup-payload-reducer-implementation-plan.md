# File Read Follow-up Payload Reducer Implementation Plan

> Implement this plan task-by-task using the available execution workflow for this repository.

**Goal:** Reduce model-facing token waste from oversized `file.read` outputs during tool-result follow-up rounds without changing raw tool output semantics.

**Architecture:** Keep `file.read` execution output and `TurnEngine` envelopes unchanged. Apply a `file.read`-specific reducer only when follow-up provider messages are assembled, and update truncation-hint logic so the prompt reflects the rendered follow-up payload instead of only the original tool result.

**Tech Stack:** Rust, serde_json, conversation follow-up assembly in `turn_shared.rs`, `turn_loop.rs`, and `turn_coordinator.rs`.

---

## Task 1: Add failing tests for file.read follow-up reduction

**Files:**
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`

**Step 1: Write the failing discovery-first follow-up test**

Add a test proving `build_turn_reply_followup_messages(...)` reduces oversized `file.read` payload summaries before the next provider round.

**Step 2: Run the test to verify RED**

Run: `cargo test -p loongclaw-app build_turn_reply_followup_messages_reduces_file_read_payload_summary -- --exact --nocapture`
Expected: FAIL because discovery-first follow-up currently forwards the raw tool result text unchanged.

**Step 3: Write the failing turn-loop tests**

Add focused tests proving both:
- `append_tool_driven_followup_messages(...)`
- `append_repeated_tool_guard_followup_messages(...)`

reduce oversized `file.read` payload summaries before generic follow-up budget truncation.

**Step 4: Run one failing turn-loop test**

Run: `cargo test -p loongclaw-app append_repeated_tool_guard_followup_messages_reduces_file_read_payload_summary -- --exact --nocapture`
Expected: FAIL because repeated-tool-guard currently only applies generic truncation.

**Step 5: Write the truncation-hint regression test**

Add a `turn_shared.rs` test proving the follow-up prompt includes the truncation hint when the rendered follow-up payload is newly marked truncated even if the original tool result was not.

## Task 2: Implement the shared follow-up reducer

**Files:**
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`

**Step 1: Add a shared follow-up reducer in `turn_shared.rs`**

Implement helpers that:
- only touch `tool_result` payloads
- only rewrite structured envelopes for `file.read`
- parse the nested `payload_summary`
- preserve `path`, `bytes`, and the original file-tool `truncated` flag
- replace large `content` with compact preview metadata
- preserve outer `payload_chars`
- set outer `payload_truncated=true` when reduction occurs
- leave non-`file.read` tool results unchanged

**Step 2: Update truncation-hint prompt logic**

Allow `build_tool_followup_user_prompt(...)` to consider both the original tool-result text and the rendered follow-up tool-result text.

**Step 3: Route discovery-first follow-up assembly through the reducer**

Update `build_turn_reply_followup_messages(...)` so follow-up provider messages map tool payloads through the reducer.

**Step 4: Route turn-loop and repeated-tool-guard follow-up assembly through the reducer**

Update both follow-up paths in `turn_loop.rs` so `file.read` reduction happens before generic follow-up budget truncation.

## Task 3: Verify and prepare GitHub delivery

**Files:**
- Modify: `docs/plans/2026-03-16-file-read-followup-payload-reducer-design.md`
- Modify: `docs/plans/2026-03-16-file-read-followup-payload-reducer-implementation-plan.md`

**Step 1: Run focused tests**

Run:
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_reduces_file_read_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app append_tool_driven_followup_messages_reduces_file_read_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app append_repeated_tool_guard_followup_messages_reduces_file_read_payload_summary -- --exact --nocapture`

Expected: PASS

**Step 2: Run adjacent regressions**

Run:
- `cargo test -p loongclaw-app tool_result_followup_tail_ -- --nocapture`
- `cargo test -p loongclaw-app tool_loop_guard_tail_ -- --nocapture`
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_ -- --nocapture`
- `cargo test -p loongclaw-app append_tool_driven_followup_messages_ -- --nocapture`
- `cargo test -p loongclaw-app append_repeated_tool_guard_followup_messages_ -- --nocapture`

Expected: PASS

**Step 3: Run repository-grade verification**

Run:
- `cargo fmt --all`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`

Expected: PASS

**Step 4: Prepare GitHub delivery**

Create or reuse a GitHub issue describing:
- why `file.read` is a follow-up token hotspot
- why the reducer is follow-up-only instead of execution-path wide
- why head-only preview was chosen over more complex preview shapes

Open a PR linked to that issue with exact validation evidence.
