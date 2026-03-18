# Shell Follow-up Payload Reducer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce model-facing token waste from oversized `shell.exec` outputs during tool-result follow-up rounds without changing raw tool output semantics.

**Architecture:** Keep `ToolCoreOutcome` and `TurnEngine` output unchanged so explicit raw-output requests still receive the original tool result envelope. Apply a shell-specific reducer only when follow-up provider messages are assembled, reusing the existing structured tool-result envelope and truncation signaling instead of adding a new transport or storage path.

**Tech Stack:** Rust, serde_json, existing conversation follow-up assembly in `turn_shared.rs`, `turn_loop.rs`, and `turn_coordinator.rs`.

---

## Task 1: Add failing tests for shell follow-up payload reduction

**Files:**
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`

**Step 1: Write the failing tests**

Add tests that prove:
- discovery-first follow-up messages reduce oversized `shell.exec` payload summaries before the next provider round
- reduced shell payloads mark `payload_truncated=true`
- `tool.search` follow-up payloads remain untouched so payload parsing semantics are preserved
- raw-output reply behavior is not part of this reducer path

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app build_turn_reply_followup_messages_reduces_shell_exec_payload_summary -- --exact --nocapture`
Expected: FAIL because discovery-first follow-up currently uses the raw tool result text unchanged.

**Step 3: Add a turn-loop failing test**

Add focused tests proving turn-loop follow-up assembly also reduces shell payloads before applying its generic payload budget, including the repeated-tool guard path that replays the latest tool context back to the provider.

**Step 4: Run test to verify it fails**

Run: `cargo test -p loongclaw-app append_tool_driven_followup_messages_reduces_shell_exec_payload_summary -- --exact --nocapture`
Expected: FAIL because the turn-loop path currently only applies generic text truncation.

## Task 2: Implement the reducer in shared follow-up assembly

**Files:**
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`

**Step 1: Add a shared shell follow-up reducer**

Implement a helper in `turn_shared.rs` that:
- only touches `[tool_result]` payloads
- only rewrites structured envelopes for `shell.exec`
- parses the envelope and nested `payload_summary`
- replaces large `stdout`/`stderr` blobs with compact previews plus counts
- preserves original `payload_chars`
- flips `payload_truncated=true` whenever reduction happened
- leaves non-shell tool results unchanged

**Step 2: Route discovery-first follow-up assembly through the reducer**

Update `build_turn_reply_followup_messages(...)` in `turn_coordinator.rs` so follow-up provider messages map tool payloads through the new reducer.

**Step 3: Route turn-loop follow-up assembly through the reducer before budget truncation**

Update `append_tool_driven_followup_messages(...)` and `append_repeated_tool_guard_followup_messages(...)` in `turn_loop.rs` so shell-specific reduction happens before the generic follow-up payload budget is applied.

**Step 4: Keep the change minimal**

Do not:
- change `shell.exec` tool execution output
- add new config surface in this slice
- add reducer behavior for `file.read`, `web.fetch`, or `tool.search`

## Task 3: Verify and document the slice

**Files:**
- Modify: `docs/plans/2026-03-16-shell-followup-payload-reducer-implementation-plan.md`

**Step 1: Run focused tests**

Run:
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_reduces_shell_exec_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app append_tool_driven_followup_messages_reduces_shell_exec_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app append_repeated_tool_guard_followup_messages_reduces_shell_exec_payload_summary -- --exact --nocapture`

Expected: PASS

**Step 2: Run adjacent regression tests**

Run:
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_include_truncation_hint_for_truncated_tool_results -- --exact --nocapture`
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_promotes_external_skill_invoke_to_system_context -- --exact --nocapture`
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_rejects_truncated_external_skill_invoke_payload -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_loop_guard_tail_ -- --nocapture`

Expected: PASS

**Step 3: Run broader package verification**

Run: `cargo test -p loongclaw-app conversation::turn_shared -- --nocapture`
Expected: PASS

**Step 4: Prepare GitHub delivery**

Create or reuse a GitHub issue describing:
- why `shell.exec` is a token hotspot
- why the reducer is follow-up-only instead of execution-path wide
- follow-on work for `file.read` and `web.fetch`

Open a PR linked to that issue with validation evidence from the commands above.
