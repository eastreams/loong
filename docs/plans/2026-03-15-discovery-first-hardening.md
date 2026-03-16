# Discovery-First Hardening Implementation Plan

**Goal:** Harden the discovery-first provider runtime by proving real provider-shape follow-up behavior, summarizing discovery-first runtime telemetry, and fixing the contract in durable architecture docs.

**Architecture:** Extend the existing discovery-first coordinator path rather than adding new infrastructure. Use real provider response shapes in coordinator tests, emit compact discovery-first conversation events only on `tool.search` turns, summarize them in analytics, and document the contract in a living design note.

**Tech Stack:** Rust, `loongclaw-app` conversation/provider/tools modules, `serde_json`, cargo test, cargo fmt, GitHub PR workflow.

---

### Task 1: Add failing discovery-first analytics summary tests

**Files:**
- Modify: `crates/app/src/conversation/analytics.rs`

**Step 1: Write the failing tests**

Add unit tests for a new discovery-first event summary function. Cover:

- counts for search rounds, follow-up requested/completed, raw-output preserved
- aggregation of `followup_added_estimated_tokens`
- search-to-invoke hit counting based on structured event payloads
- ignoring lookalike events or malformed payloads

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app summarize_discovery_first_events -- --nocapture`
Expected: FAIL because the new summary type and parser do not exist yet.

**Step 3: Write minimal implementation**

Add a discovery-first summary type and parser in `analytics.rs` that reads only
the new event family and produces the small rollup needed by the coordinator
tests.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app summarize_discovery_first_events -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/analytics.rs
git commit -m "feat: summarize discovery-first telemetry"
```

### Task 2: Add failing provider-shape end-to-end follow-up tests

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add coordinator tests that:

- build the first-round `ProviderTurn` via `extract_provider_turn_with_scope(...)`
- use real JSON bodies for:
  - OpenAI chat-completions tool calls
  - Responses function calls
  - Anthropic `tool_use`
  - Bedrock `toolUse`
  - inline function blocks
- queue a second provider turn that requests a discoverable tool by its native
  name
- assert:
  - two provider turn requests occurred
  - the second request received `[tool_result]` follow-up context
  - the second-turn native tool name rewrites to `tool.invoke`
  - final reply or raw output comes from the invoked tool, not the search card
- add at least one assertion that the persisted discovery-first events summarize
  the follow-up behavior correctly

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app provider_shape_tool_search -- --nocapture`
Expected: FAIL because the helper coverage and telemetry assertions do not exist
yet.

**Step 3: Write minimal implementation**

Add compact fixture helpers if needed, but keep the real coordinator path
unchanged until the telemetry seam is wired.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app provider_shape_tool_search -- --nocapture`
Expected: PASS with the real follow-up loop exercised.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/tests.rs
git commit -m "test: cover provider-shape discovery followups"
```

### Task 3: Implement discovery-first coordinator telemetry

**Files:**
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Modify: `crates/app/src/conversation/analytics.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Reuse the new analytics and coordinator tests so they fail on missing telemetry
events / summaries.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app summarize_discovery_first_events provider_shape_tool_search -- --nocapture`
Expected: FAIL because no discovery-first events are emitted or summarized.

**Step 3: Write minimal implementation**

Emit compact discovery-first conversation events from the provider follow-up
path when a turn contains `tool.search`. Include:

- whether follow-up was requested
- the follow-up result outcome
- whether raw-output mode preserved the follow-up
- the follow-up tool name
- whether it resolved to `tool.invoke`
- initial / follow-up / added estimated tokens

Add analytics parsing and re-exports as needed.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app summarize_discovery_first_events provider_shape_tool_search -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/mod.rs crates/app/src/conversation/analytics.rs crates/app/src/conversation/tests.rs
git commit -m "feat: emit discovery-first followup telemetry"
```

### Task 4: Add the long-lived architecture note

**Files:**
- Create: `docs/design-docs/discovery-first-tool-runtime-contract.md`
- Modify: `docs/design-docs/index.md`
- Modify: `docs/design-docs/provider-runtime-roadmap.md`

**Step 1: Write the failing test**

No automated test. Use doc review against the approved design.

**Step 2: Review expected contract coverage**

Confirm the note covers:

- provider-core tools
- discoverable tools and parser rewrite rules
- lease/session/turn scope
- follow-up provider turn requirement
- raw-output behavior
- unsupported patterns

**Step 3: Write minimal implementation**

Add the new note, link it from the design-doc index, and add a short pointer
from the provider runtime roadmap.

**Step 4: Review docs**

Read the changed docs for accuracy and consistency with the implementation.

**Step 5: Commit**

```bash
git add docs/design-docs/discovery-first-tool-runtime-contract.md docs/design-docs/index.md docs/design-docs/provider-runtime-roadmap.md
git commit -m "docs: add discovery-first runtime contract note"
```

### Task 5: Verification and GitHub packaging

**Files:**
- Modify: GitHub artifacts only if needed

**Step 1: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app summarize_discovery_first_events -- --nocapture
cargo test -p loongclaw-app provider_shape_tool_search -- --nocapture
```

Expected: PASS.

**Step 2: Run broader verification**

Run:

```bash
cargo fmt --all --check
cargo test -p loongclaw-app -- --test-threads=1
```

Expected: PASS.

**Step 3: Inspect isolation**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Expected: only hardening-scope files are present.

**Step 4: Push and update PR**

Push the branch and update existing issue / PR copy in English, explicitly
describing:

- provider-shape discovery-first e2e coverage
- discovery-first telemetry summary
- durable runtime contract note

**Step 5: Clean branch-local build artifacts**

Remove any branch-local `target/` output before reporting completion.
