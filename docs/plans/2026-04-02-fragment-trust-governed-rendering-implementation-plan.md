# Fragment Trust And Governed Prompt Rendering Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make prompt trust a first-class part of prompt orchestration by adding fragment render policy and a shared governed advisory renderer, then route discovery-delta and advisory memory through that shared trust boundary.

**Architecture:** Keep the current typed prompt fragment topology and single-system-message provider contract. Add one narrow render-policy enum to `PromptFragment`, one shared governed advisory rendering helper owned by prompt orchestration, and migrate existing advisory call sites away from bespoke sanitization logic onto that shared path.

**Tech Stack:** Rust, LoongClaw conversation runtime, prompt fragments, advisory prompt rendering, unit tests, conversation runtime tests, cargo fmt, clippy.

---

## Implementation Tasks

### Task 1: Write the failing trust-boundary tests

**Files:**
- Modify: `crates/app/src/conversation/prompt_orchestrator.rs`
- Modify: `crates/app/src/conversation/tool_discovery_state.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Add a failing compiler policy test**

Add a prompt-orchestrator test that constructs one trusted fragment and one
governed advisory fragment containing heading-shaped advisory content.

Assert that compilation preserves the trusted fragment verbatim while demoting
governed advisory headings.

**Step 2: Add a failing discovery-delta renderer test**

Add a `ToolDiscoveryState` test with prompt-shaped `query`, `summary`,
`search_hint`, and `diagnostics.reason` values that try to create headings,
fake tool-call content, or fake bracketed sections.

Assert that the rendered delta stays readable but only through the shared
advisory inline renderer contract.

**Step 3: Add a failing advisory-memory reuse test**

Add a provider runtime test proving advisory memory rendering uses the same
governed heading behavior as prompt fragments.

Use a memory entry with governed headings and verify the preserved root heading
and demoted nested headings match the shared policy.

**Step 4: Add a failing end-to-end runtime test**

Add or extend a conversation runtime test that injects advisory content shaped
like fake system sections or fake tool instructions and asserts the compiled
prompt does not surface raw spoofed structure.

**Step 5: Run the red tests**

Run:

```bash
cargo test -p loongclaw-app prompt_orchestrator -- --nocapture
cargo test -p loongclaw-app tool_discovery_state -- --nocapture
cargo test -p loongclaw-app append_advisory_memory_message -- --nocapture
cargo test -p loongclaw-app tool_discovery_delta -- --nocapture
```

Expected:
- the new compiler-policy and shared-renderer expectations fail before
  implementation

### Task 2: Add fragment render policy and shared governed renderer

**Files:**
- Create or modify: `crates/app/src/conversation/prompt_rendering.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Modify: `crates/app/src/conversation/prompt_fragments.rs`
- Modify: `crates/app/src/conversation/prompt_orchestrator.rs`
- Modify: `crates/app/src/advisory_prompt.rs`

**Step 1: Introduce the render policy enum**

Add a narrow render policy to `PromptFragment`.

Start with only the variants needed for this issue:

- trusted literal rendering
- governed advisory rendering with allowed root headings

Do not add speculative policy variants.

**Step 2: Introduce one shared governed rendering helper**

Implement the conversation-owned helper that:

- demotes governed advisory headings at fragment scope
- preserves one allowed container heading when explicitly configured
- exposes a shared inline advisory value renderer for embedded untrusted values

Prefer reusing the existing advisory-heading logic instead of duplicating it.

**Step 3: Apply render policy in prompt compilation**

Update `PromptCompiler::compile(...)` so fragment content is rendered according
to policy before final system-text assembly.

Preserve existing trim, dedupe, and lane ordering behavior.

### Task 3: Migrate current advisory surfaces onto the shared boundary

**Files:**
- Modify: `crates/app/src/conversation/tool_discovery_state.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Modify: `crates/app/src/conversation/turn_middleware.rs`

**Step 1: Move discovery inline-value sanitization onto the shared helper**

Replace `ToolDiscoveryState` local advisory value sanitization with calls into
the shared governed renderer module.

Keep `ToolDiscoveryState` responsible for section structure only.

**Step 2: Mark discovery fragments as governed advisory**

When creating `PromptLane::ToolDiscoveryDelta` fragments, assign the advisory
render policy explicitly.

Keep current dedupe key, lane, and filtered tool-view behavior.

**Step 3: Route advisory memory through the shared helper**

Update `append_advisory_memory_message(...)` to use the same shared governed
renderer module rather than owning a separate sanitization path.

Preserve the current allowed-root-heading behavior for session profile, memory
summary, and durable recall containers.

### Task 4: Verify the touched surface

**Files:**
- Verify only

**Step 1: Run targeted tests**

```bash
cargo test -p loongclaw-app prompt_orchestrator -- --nocapture
cargo test -p loongclaw-app tool_discovery_state -- --nocapture
cargo test -p loongclaw-app append_advisory_memory_message -- --nocapture
cargo test -p loongclaw-app tool_discovery_delta -- --nocapture
cargo test -p loongclaw-app default_runtime_build_context_sanitizes_tool_discovery_delta_advisory_text -- --nocapture
```

**Step 2: Run adjacent prompt/runtime tests**

```bash
cargo test -p loongclaw-app default_runtime_build_context_includes_tool_discovery_delta_from_persisted_state -- --nocapture
cargo test -p loongclaw-app default_runtime_build_messages_filters_tool_discovery_delta_to_requested_tool_view -- --nocapture
cargo test -p loongclaw-app message_builder_keeps_durable_recall_advisory_when_memory_files_look_like_identity -- --nocapture
```

**Step 3: Run format and lint**

```bash
cargo fmt --all -- --check
cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings
```

### Task 5: Prepare clean delivery

**Files:**
- Modify: `docs/plans/2026-04-02-fragment-trust-governed-rendering-design.md`
- Modify: `docs/plans/2026-04-02-fragment-trust-governed-rendering-implementation-plan.md`
- Modify: `crates/app/src/conversation/prompt_fragments.rs`
- Modify: `crates/app/src/conversation/prompt_orchestrator.rs`
- Modify: `crates/app/src/conversation/tool_discovery_state.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/advisory_prompt.rs`

**Step 1: Inspect scope before commit**

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

**Step 2: Commit the design and implementation slice**

```bash
git add docs/plans/2026-04-02-fragment-trust-governed-rendering-design.md
git add docs/plans/2026-04-02-fragment-trust-governed-rendering-implementation-plan.md
git add crates/app/src/conversation/prompt_fragments.rs
git add crates/app/src/conversation/prompt_orchestrator.rs
git add crates/app/src/conversation/tool_discovery_state.rs
git add crates/app/src/provider/request_message_runtime.rs
git add crates/app/src/conversation/tests.rs
git add crates/app/src/advisory_prompt.rs
git commit -m "feat(app): govern advisory prompt fragment rendering"
```
