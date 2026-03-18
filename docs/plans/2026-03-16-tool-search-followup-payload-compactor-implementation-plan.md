# Tool Search Follow-up Payload Compactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce follow-up token waste from `tool.search` tool results without changing raw execution output or breaking discovery lease bridging.

**Architecture:** Keep `tool.search` execution output and `TurnEngine` envelopes unchanged. Add a follow-up-only structural compactor that rewrites `tool.search` payload summaries in conversation follow-up assembly while preserving bridge-required fields and leaving `payload_truncated` untouched.

**Tech Stack:** Rust, serde_json, conversation follow-up assembly in `turn_shared.rs`, `turn_coordinator.rs`, `turn_loop.rs`, and provider bridge validation in `provider/shape.rs`.

---

### Task 1: Add failing tests for bridge-safe tool.search compaction

**Files:**
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/provider/shape.rs`

**Step 1: Write the failing discovery-first follow-up test**

Add a test proving `build_turn_reply_followup_messages(...)` compacts an oversized `tool.search` payload summary before the next provider round while keeping `payload_truncated=false`.

**Step 2: Run the test to verify RED**

Run: `cargo test -p loongclaw-app build_turn_reply_followup_messages_compacts_tool_search_payload_summary -- --exact --nocapture`
Expected: FAIL because discovery-first follow-up currently forwards the raw `tool.search` payload unchanged.

**Step 3: Write the failing turn-loop tests**

Add focused tests proving both:
- `append_tool_driven_followup_messages(...)`
- `append_repeated_tool_guard_followup_messages(...)`

compact `tool.search` payload summaries before generic follow-up budget truncation.

**Step 4: Run one failing turn-loop test**

Run: `cargo test -p loongclaw-app append_tool_driven_followup_messages_compacts_tool_search_payload_summary -- --exact --nocapture`
Expected: FAIL because turn-loop follow-up currently forwards the raw `tool.search` payload unchanged.

**Step 5: Write the provider bridge safety regression test**

Add a provider bridge test proving a compacted `tool.search` follow-up payload still exposes `tool_id` and `lease` well enough for lease reconstruction.

### Task 2: Implement the minimal follow-up compactor

**Files:**
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`

**Step 1: Add a shared `tool.search` follow-up compactor in `turn_shared.rs`**

Implement helpers that:
- only touch `tool_result` payloads
- only rewrite structured envelopes for `tool.search`
- parse the nested `payload_summary`
- preserve bridge-safe fields for every result
- prune low-value search explanation metadata
- preserve outer `payload_chars`
- avoid setting `payload_truncated=true`
- leave non-`tool.search` payloads unchanged

**Step 2: Route discovery-first follow-up assembly through the compactor**

Update `build_turn_reply_followup_messages(...)` so provider follow-up messages compact `tool.search` payloads before sending them back to the model.

**Step 3: Route turn-loop and repeated-tool-guard follow-up assembly through the compactor**

Update both follow-up paths in `turn_loop.rs` so search compaction happens before generic follow-up budget truncation.

### Task 3: Verify and prepare GitHub delivery

**Files:**
- Modify: `docs/plans/2026-03-16-tool-search-followup-payload-compactor-design.md`
- Modify: `docs/plans/2026-03-16-tool-search-followup-payload-compactor-implementation-plan.md`

**Step 1: Run focused tests**

Run:
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_compacts_tool_search_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app append_tool_driven_followup_messages_compacts_tool_search_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app append_repeated_tool_guard_followup_messages_compacts_tool_search_payload_summary -- --exact --nocapture`
- `cargo test -p loongclaw-app bridge_context_accepts_compacted_search_results -- --exact --nocapture`

Expected: PASS

**Step 2: Run adjacent regressions**

Run:
- `cargo test -p loongclaw-app build_turn_reply_followup_messages_ -- --nocapture`
- `cargo test -p loongclaw-app append_tool_driven_followup_messages_ -- --nocapture`
- `cargo test -p loongclaw-app append_repeated_tool_guard_followup_messages_ -- --nocapture`
- `cargo test -p loongclaw-app bridge_context_ -- --nocapture`

Expected: PASS

**Step 3: Run repository-grade verification**

Run:
- `cargo fmt --all`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`

Expected: PASS

**Step 4: Prepare GitHub delivery**

Create a new GitHub issue describing:
- why `tool.search` is a follow-up token hotspot
- why this slice uses structural compaction instead of truncation
- why `payload_truncated` must stay untouched for bridge safety

Open a PR linked to that issue with exact validation evidence.
