# Tool-Discovery Prompt Boundary Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Harden discovery-delta prompt rendering and tool-search summary metadata so advisory `tool.search` state stays useful without regaining system-prompt authority.

**Architecture:** Keep the existing discovery-state and prompt-fragment topology. Add failing tests first, then implement one narrow advisory-text rendering guard inside `ToolDiscoveryState`, plus the smallest compaction fix needed to preserve immediate follow-up metadata while leaving persisted discovery state lease-free.

**Tech Stack:** Rust, LoongClaw conversation runtime, prompt fragments, `tool.search`, focused unit tests, cargo fmt, clippy, workspace tests.

---

## Implementation Tasks

### Task 1: Write the failing discovery-boundary tests

**Files:**
- Modify: `crates/app/src/conversation/tool_discovery_state.rs`
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/conversation/tool_result_compaction.rs`

**Step 1: Add a failing prompt-sanitization test**

Create a `ToolDiscoveryState` test with newline-heavy `query`, `summary`,
`search_hint`, and `diagnostics.reason` values.

Assert that `render_delta_prompt()` keeps the data visible but does not project
raw newlines or heading-shaped fragments into the system prompt.

**Step 2: Add a failing results-only state-recovery test**

Create a `ToolDiscoveryState::from_tool_search_payload(...)` test with a payload
that has only `results`.

Assert that the advisory state is still recovered.

**Step 3: Add a failing compaction test**

Create a compaction test that feeds a `tool.search` payload with `lease`,
`diagnostics`, and `exact_tool_id`.

Assert that the compacted payload preserves `lease` for the live follow-up
provider round while also preserving advisory metadata such as `exact_tool_id`
and `diagnostics`.

**Step 4: Run targeted tests to confirm red**

Run:

```bash
cargo test -p loongclaw-app tool_discovery_state -- --nocapture
cargo test -p loongclaw-app compact_tool_search_payload -- --nocapture
```

Expected:
- the new sanitization, results-only, and compaction expectations fail before
  implementation

### Task 2: Implement the minimal boundary fix

**Files:**
- Modify: `crates/app/src/conversation/tool_discovery_state.rs`
- Modify: `crates/app/src/conversation/tool_result_compaction.rs`

**Step 1: Treat results-only payloads as valid discovery state**

Update `from_tool_search_payload(...)` so non-empty normalized entries count as
state.

**Step 2: Add one small advisory-text sanitizer**

Implement a helper local to `tool_discovery_state.rs` that converts untrusted
discovery text into a safe, single-line advisory representation before prompt
rendering.

**Step 3: Apply the sanitizer at every prompt-rendered field**

Cover:

- `query`
- `diagnostics.reason`
- `summary`
- `search_hint`
- `argument_hint`
- `required_fields`
- `required_field_groups`
- `exact_tool_id` refresh rendering when needed

Keep tool ids and refresh guidance readable.

**Step 4: Preserve lease and missing top-level metadata in compacted payload summaries**

Update result compaction so the model-facing compact summary keeps the current
tool-card `lease` together with top-level advisory metadata that matters for the
immediate follow-up turn.

Do not change the separate persisted discovery-event behavior that already strips
leases.

### Task 3: Verify the touched surface

**Files:**
- Verify only

**Step 1: Run targeted tests**

```bash
cargo test -p loongclaw-app tool_discovery_state -- --nocapture
cargo test -p loongclaw-app compact_tool_search_payload -- --nocapture
```

**Step 2: Run adjacent conversation tests**

```bash
cargo test -p loongclaw-app tool_discovery_delta -- --nocapture
cargo test -p loongclaw-app default_runtime_build_context_uses_configured_runtime_tool_view_for_tool_discovery_delta -- --nocapture
cargo test -p loongclaw-app default_runtime_kernel_build_context_uses_configured_runtime_tool_view_for_tool_discovery_delta -- --nocapture
```

**Step 3: Run format and lint**

```bash
cargo fmt --all -- --check
cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings
```

### Task 4: Run full verification and prepare clean delivery

**Files:**
- Verify only

**Step 1: Run full tests**

```bash
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

**Step 2: Run architecture and mirror checks**

```bash
LOONGCLAW_ARCH_STRICT=true scripts/check_architecture_boundaries.sh
scripts/check_dep_graph.sh
diff CLAUDE.md AGENTS.md
```

**Step 3: Inspect scope before commit**

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

**Step 4: Commit**

```bash
git add docs/plans/2026-04-01-tool-discovery-prompt-boundary-design.md
git add docs/plans/2026-04-01-tool-discovery-prompt-boundary-implementation-plan.md
git add crates/app/src/conversation/tool_discovery_state.rs
git add crates/app/src/conversation/tool_result_compaction.rs
git add crates/app/src/conversation/tests.rs
git commit -m "fix(app): harden tool discovery prompt boundaries"
```
