# Memory Layered Continuity Foundation Implementation Plan

**Goal:** Turn LoongClaw's current memory foundation into a usable continuity
product by adding bounded hot memory and episodic recall while preserving
LoongClaw-owned canonical history and final context projection.

**Architecture:** Reuse the current canonical SQLite history and typed record
model. Add a builtin derived-artifact layer for durable hot memory, expose a
builtin episodic recall path over canonical history, and keep
`ConversationContextEngine` as the final projection seam. Do not make Markdown
files, vector stores, or external memory vendors the new source of truth.

**Tech Stack:** Rust, SQLite/FTS5, existing `loongclaw-app`, `loongclaw-daemon`,
and `loongclaw-kernel` memory seams, current conversation/context runtime, Rust
unit and integration tests.

**Why this slice first:**

- Close the highest-impact product gaps without an architectural rewrite.
- Keep debugging simple by avoiding premature semantic-vendor integration.
- Create the right substrate for later compaction flush, procedural memory, and
  external adapters.

---

## Task 1: Add Typed Builtin Derived-Memory Artifacts

**Files:**
- Create: `crates/app/src/memory/artifacts.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/memory/protocol.rs`
- Modify: `crates/contracts/src/memory_types.rs`
- Modify: `crates/app/src/memory/sqlite.rs`
- Test: `crates/app/src/memory/artifacts.rs`
- Test: `crates/app/src/memory/sqlite.rs`

**Step 1: Write the failing test**

Add tests that assert:

- builtin derived-memory kinds exist for at least `profile` and `fact`
- artifacts preserve `scope`, `content`, and structured metadata
- artifact persistence round-trips through SQLite
- artifact source-record provenance is retained when supplied

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app memory::artifacts -- --nocapture`

Expected: FAIL because there is no typed artifact layer yet.

**Step 3: Write minimal implementation**

Add:

- `DerivedMemoryKind`
- `DerivedMemoryRecord`
- SQLite tables for derived artifacts plus optional source-record links
- memory-core operations for builtin artifact create/list/remove behavior

Keep the first slice narrow:

- no embeddings
- no external backends
- no automatic derivation yet

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app memory::artifacts -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/memory/artifacts.rs crates/app/src/memory/mod.rs crates/app/src/memory/protocol.rs crates/contracts/src/memory_types.rs crates/app/src/memory/sqlite.rs
git commit -m "feat(memory): add builtin derived artifact storage"
```

## Task 2: Add Bounded Hot-Memory Projection

**Files:**
- Modify: `crates/app/src/config/memory.rs`
- Modify: `crates/app/src/memory/runtime_config.rs`
- Modify: `crates/app/src/memory/context.rs`
- Modify: `crates/app/src/memory/orchestrator.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Test: `crates/app/src/memory/context.rs`
- Test: `crates/app/src/memory/orchestrator.rs`

**Step 1: Write the failing test**

Add tests that assert:

- hot memory can be projected into prompt context ahead of the recent window
- projection respects explicit byte/char budgets
- recent window behavior remains unchanged when no hot memory exists
- projection order is deterministic across repeated reads

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app hot_memory_projection -- --nocapture`

Expected: FAIL because prompt hydration only knows about `profile_note`,
deterministic summary, and recent turns.

**Step 3: Write minimal implementation**

Add:

- builtin projection budgets for hot memory
- ordered prompt projection for derived `profile` / `fact` artifacts
- deterministic diagnostics showing projected artifact counts and degradation
  state

Do not add session-frozen snapshots in this slice. LoongClaw does not yet have a
provider-cache lifecycle that justifies a more complex snapshot invalidation
mechanism.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app hot_memory_projection -- --nocapture`

Run: `cargo test -p loongclaw-app hydrated_memory -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/config/memory.rs crates/app/src/memory/runtime_config.rs crates/app/src/memory/context.rs crates/app/src/memory/orchestrator.rs crates/app/src/provider/request_message_runtime.rs
git commit -m "feat(memory): project bounded hot memory into context"
```

## Task 3: Add Agent-Facing Durable Memory Management

**Files:**
- Modify: `crates/app/src/tools/mod.rs`
- Create: `crates/app/src/tools/memory_manage.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Test: `crates/app/src/tools/memory_manage.rs`

**Step 1: Write the failing test**

Add tests that assert:

- a tool can add a durable builtin memory artifact
- replace/remove flows work without artifact-id hardcoding
- duplicate or ambiguous replace targets fail with clear errors
- artifact writes remain scoped and auditable

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app memory_manage -- --nocapture`

Expected: FAIL because there is no operator/model-facing durable memory tool.

**Step 3: Write minimal implementation**

Add a dedicated tool surface that manages durable builtin memory artifacts.

Recommended behavior:

- `add`
- `replace`
- `remove`
- target classes limited to builtin hot-memory artifacts in this slice
- substring or selector matching allowed, but without forcing opaque internal
  ids into the prompt

Do not overload transcript/session tools with this responsibility.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app memory_manage -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/mod.rs crates/app/src/tools/memory_manage.rs crates/app/src/tools/runtime_config.rs
git commit -m "feat(tools): add durable memory management surface"
```

## Task 4: Add Episodic Recall Over Canonical History

**Files:**
- Modify: `crates/app/src/memory/sqlite.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/tools/session.rs`
- Test: `crates/app/src/memory/sqlite.rs`
- Test: `crates/app/src/tools/session.rs`

**Step 1: Write the failing test**

Add tests that assert:

- canonical records can be queried through SQLite FTS5 by content
- results include session provenance and canonical kind metadata
- retrieval is bounded by limit and snippet budget
- visibility rules still apply when searching across sessions

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app session_search -- --nocapture`

Expected: FAIL because there is no episodic recall path yet.

**Step 3: Write minimal implementation**

Add:

- FTS5 index over searchable canonical record content
- bounded episodic recall query path
- a session- or memory-oriented tool surface that returns snippets, kind, scope,
  session id, and basic scores

Keep the first slice deterministic:

- no embeddings
- no hybrid ranking
- no cross-vendor dependencies

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app session_search -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/memory/sqlite.rs crates/app/src/memory/mod.rs crates/app/src/tools/session.rs
git commit -m "feat(memory): add episodic recall over canonical history"
```

## Task 5: Add Diagnostics, Docs, And Follow-On Guards

**Files:**
- Modify: `docs/ROADMAP.md`
- Modify: `docs/design-docs/index.md`
- Modify: `crates/daemon/src/main.rs` if new diagnostics are surfaced
- Modify: `crates/daemon/src/lib.rs` if new diagnostics are surfaced
- Test: relevant docs and daemon tests

**Step 1: Write the failing test**

Add tests that assert:

- runtime diagnostics surface hot-memory and episodic-recall capability state
- new tools or diagnostics remain discoverable through daemon/CLI surfaces

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon memory -- --nocapture`

Expected: FAIL if no diagnostics or listings expose the new builtin capabilities.

**Step 3: Write minimal implementation**

Add:

- operator-facing diagnostics for hot-memory saturation, projected artifact
  counts, and episodic-recall availability
- docs updates that explain scope and non-goals

Document the intentional non-goals:

- no automatic LLM memory extraction yet
- no compaction flush yet
- no procedural-memory convergence yet
- no external memory adapters yet

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon memory -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add docs/ROADMAP.md docs/design-docs/index.md
# If this slice adds new diagnostics, also stage:
# git add crates/daemon/src/main.rs crates/daemon/src/lib.rs
git commit -m "docs(memory): document layered continuity foundation"
```

## Acceptance Criteria

- LoongClaw can persist builtin durable hot-memory artifacts separately from the
  recent transcript window.
- Prompt hydration can project bounded hot memory plus recent context without
  disturbing current recent-window semantics.
- Operators or the model can manage builtin durable memory through a dedicated
  tool surface.
- The runtime can search canonical history through a bounded builtin episodic
  recall path.
- All new layers remain subordinate to LoongClaw-owned canonical history and
  final prompt projection.

## Explicit Non-Goals

- No vector or hybrid retrieval in the first slice
- No memory-vendor integration in the first slice
- No compaction-time auto-flush in the first slice
- No procedural-memory and skills convergence in the first slice
- No topic-router or multi-space memory UX in the first slice

## Validation

At the end of implementation, run at minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

If the slice touches docs, roadmap entries, or generated references, also run:

```bash
scripts/check-docs.sh
git diff --check
```

If any tool or test requires process-global state mutation, serialize the
relevant test scope and document the isolation strategy in the PR.
