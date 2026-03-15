# Issue 128 Approval Attention Rebuild Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rebuild the governed approval request lifecycle and operator attention surface for issue
`#128` on current `alpha-test`, including durable request/grant persistence, operator resolution,
automatic replay, and unified attention summaries.

**Architecture:** Extend the SQLite-backed `SessionRepository` with explicit approval request and
grant records, materialize approval-required tool calls from `TurnEngine`, expose a narrow
approval app-tool surface, and derive a canonical attention union view from lifecycle execution and
grant audit state.

**Tech Stack:** Rust, Tokio, serde/serde_json, rusqlite, existing conversation runtime and session
tool infrastructure, cargo test, cargo fmt, cargo clippy.

---

### Task 1: Lock durable approval storage with failing repository tests

**Files:**
- Modify: `crates/app/src/memory/sqlite.rs`
- Modify: `crates/app/src/session/repository.rs`

**Step 1: Write the failing tests**

Add repository tests that expect:

- `approval_requests` persists a new pending request
- duplicate create for the same deterministic request ID returns the same row
- request status transitions are conditional and explicit
- `approval_grants` persists a session-scoped runtime grant

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app approval_request_repository_ -- --nocapture --test-threads=1
```

Expected: FAIL because approval storage does not exist yet.

**Step 3: Write minimal implementation**

Add:

- SQLite schema for `approval_requests`
- SQLite schema for `approval_grants`
- repository structs and methods for create/load/list/transition request records
- repository methods for upsert/load grant records

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app approval_request_repository_ -- --nocapture --test-threads=1
```

Expected: PASS.

### Task 2: Materialize approval-required governed tool calls in `TurnEngine`

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- a governed tool call that requires approval creates a durable pending request
- repeated blocking of the same call reuses the deterministic request ID
- the turn result carries structured approval requirement data including request ID, tool name,
  approval key, reason, and rule ID
- the turn loop renders a truthful approval-required reply containing the request ID

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app governed_tool_approval_request_ turn_loop_renders_tool_approval_required -- --nocapture --test-threads=1
```

Expected: FAIL because approval-required lifecycle support does not exist.

**Step 3: Write minimal implementation**

Add:

- structured approval requirement/result types
- deterministic request ID generation
- approval request persistence on the governed-tool approval path
- turn-loop rendering that surfaces the request ID

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app governed_tool_approval_request_ turn_loop_renders_tool_approval_required -- --nocapture --test-threads=1
```

Expected: PASS.

### Task 3: Add failing approval tool query tests

**Files:**
- Create: `crates/app/src/tools/approval.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- `approval_requests_list` returns only visible approval requests
- `approval_request_status` returns a full request snapshot for a visible request
- hidden requests are rejected
- query responses include source-specific attention blocks and a canonical union block

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app approval_request_tool_query_ -- --nocapture --test-threads=1
```

Expected: FAIL because the approval tool surface does not exist.

**Step 3: Write minimal implementation**

Add:

- approval tool module and catalog descriptors
- list and status handlers with session visibility enforcement
- request JSON rendering that includes execution/grant/union attention structures

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app approval_request_tool_query_ -- --nocapture --test-threads=1
```

Expected: PASS.

### Task 4: Add failing approval resolution tests for deny and replay flows

**Files:**
- Modify: `crates/app/src/tools/approval.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- `approval_request_resolve` with `deny` moves the request to `denied`
- non-pending requests fail closed on duplicate resolve
- `approve_once` resumes the original blocked tool call and records lifecycle events
- resumed execution failure records `last_error` and attention-worthy evidence
- `approve_always` writes a lineage-scoped runtime grant and replays the request

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app approval_request_resolve_ approval_request_resume_ approval_request_always_grant_ -- --nocapture --test-threads=1
```

Expected: FAIL because resolve/replay support is not wired yet.

**Step 3: Write minimal implementation**

Add:

- runtime-backed `approval_request_resolve`
- conditional transitions through `pending -> approved/denied -> executing -> executed`
- runtime grant persistence for `approve_always`
- replay support for the original blocked tool call without re-entering the approval-required path
- explicit lifecycle events for resolve, replay start, replay success, and replay failure

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app approval_request_resolve_ approval_request_resume_ approval_request_always_grant_ -- --nocapture --test-threads=1
```

Expected: PASS.

### Task 5: Add failing operator attention summary and filter tests

**Files:**
- Modify: `crates/app/src/tools/approval.rs`
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/session/repository.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- list responses include unified `attention_summary`
- attention summaries separate execution-only, grant-only, and combined hotspots
- grant-side filters work explicitly alongside status/session filters
- per-request status exposes canonical attention signals with source tags

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app approval_request_attention_ approval_request_tool_query_list_ -- --nocapture --test-threads=1
```

Expected: FAIL because the current list/status responses do not derive canonical attention.

**Step 3: Write minimal implementation**

Add:

- execution-side attention derivation
- grant-side attention derivation
- canonical union attention block
- list-level summaries and hotspot counts by reason/action/tool/session
- explicit grant attention filters and stable validation errors for unsupported filter values

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app approval_request_attention_ approval_request_tool_query_list_ -- --nocapture --test-threads=1
```

Expected: PASS.

### Task 6: Refresh docs and capability snapshots if the approval tool surface changes them

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: relevant docs only if test failures or snapshot expectations require it

**Step 1: Write the failing test**

Add or update tests that assert:

- approval tools appear in runtime tool snapshots only when available
- provider-facing capability descriptions remain consistent with the new tool surface

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app tool_snapshot approval_request_tool -- --nocapture --test-threads=1
```

Expected: FAIL if the exposed surface is not reflected in catalog/snapshot outputs.

**Step 3: Write minimal implementation**

Update tool catalog definitions, visibility-driven snapshots, and any necessary docs text so the
advertised tool surface matches runtime behavior.

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app tool_snapshot approval_request_tool -- --nocapture --test-threads=1
```

Expected: PASS.

### Task 7: Run full verification and prepare GitHub delivery

**Files:**
- Review only the files touched above

**Step 1: Run formatting**

Run:

```bash
cargo fmt --all -- --check
```

Expected: PASS.

**Step 2: Run clippy**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 3: Run full tests**

Run:

```bash
cargo test --workspace --all-features
```

Expected: PASS.

**Step 4: Inspect the exact diff**

Run:

```bash
git status --short
git diff --stat
git diff
```

Expected: only approval-lifecycle/attention rebuild files plus any required docs/tests.

**Step 5: Commit**

Create clean task-scoped commits after verifying the diff is isolated.

**Step 6: Push and open PR**

Push to `fork-chumyin`, open a new PR against `loongclaw-ai/loongclaw:alpha-test`, follow the
repository PR template, and include `Closes #128` if the final implementation fully resolves the
issue.
