# LoongClaw Pluggable Memory Systems Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a phased, backward-compatible architecture foundation for user-selectable memory systems while preserving LoongClaw-owned canonical history and the current built-in SQLite baseline.

**Architecture:** Introduce a `MemorySystem` selection surface and metadata first, then evolve the current memory module into a richer memory orchestrator that separates canonical persistence, derivation, retrieval, and context projection. Keep `ConversationContextEngine` as the final prompt projection seam and keep ACP feeding the same canonical fact layer without reusing provider-only lifecycle hooks.

**Tech Stack:** Rust, serde/toml config, the existing `loongclaw-app` and `loongclaw-daemon` crates, SQLite-backed memory, async traits, registry patterns already used by `context_engine_registry`, and Rust unit/integration tests.

---

### Task 1: Add Memory-System Domain Types

**Files:**
- Create: `crates/app/src/memory/system.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/config/tools_memory.rs`
- Test: `crates/app/src/memory/system.rs`

**Step 1: Write the failing test**

Add tests that assert:

- built-in memory system metadata exists
- memory-system ids normalize predictably
- default config resolves to `system = "builtin"`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app memory_system -- --nocapture`

Expected: FAIL because `MemorySystemMetadata` and related helpers do not exist.

**Step 3: Write minimal implementation**

Add:

- `MemorySystemSelection`
- `MemorySystemMetadata`
- `MemorySystemCapability`
- config field `memory.system`
- default built-in system id

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app memory_system -- --nocapture`

Expected: PASS for the new metadata and config-resolution tests.

**Step 5: Commit**

```bash
git add crates/app/src/memory/system.rs crates/app/src/memory/mod.rs crates/app/src/config/tools_memory.rs
git commit -m "feat: add memory system metadata foundation"
```

### Task 2: Add Memory-System Registry And Diagnostics

**Files:**
- Create: `crates/app/src/memory/system_registry.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`
- Test: `crates/app/src/memory/system_registry.rs`

**Step 1: Write the failing test**

Add tests that assert:

- built-in system is always registered
- unknown system ids fail with a useful error
- metadata listing is stable and sorted

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app system_registry -- --nocapture`

Expected: FAIL because the registry does not exist.

**Step 3: Write minimal implementation**

Mirror the existing context-engine registry pattern:

- `register_memory_system`
- `resolve_memory_system`
- `list_memory_system_metadata`
- `memory_system_id_from_env`

Expose a daemon diagnostic such as `list-memory-systems`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app system_registry -- --nocapture`

Run: `cargo test -p loongclaw-daemon list_memory_systems -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/memory/system_registry.rs crates/app/src/memory/mod.rs crates/app/src/lib.rs crates/daemon/src/main.rs
git commit -m "feat: add memory system registry and diagnostics"
```

### Task 3: Introduce A Typed Hydrated-Memory Snapshot

**Files:**
- Create: `crates/app/src/memory/orchestrator.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/memory/runtime_config.rs`
- Test: `crates/app/src/memory/orchestrator.rs`

**Step 1: Write the failing test**

Add tests that assert:

- built-in orchestrator returns recent window records
- built-in orchestrator returns deterministic diagnostics
- built-in orchestrator preserves current summary/profile behavior

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app hydrated_memory -- --nocapture`

Expected: FAIL because the typed orchestrator and snapshot do not exist.

**Step 3: Write minimal implementation**

Add:

- `HydratedMemoryContext`
- `MemoryDiagnostics`
- built-in orchestrator implementation that wraps current prompt hydration
- runtime config plumbing for `memory.system`, `memory.fail_open`, and
  `memory.ingest_mode`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app hydrated_memory -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/memory/orchestrator.rs crates/app/src/memory/mod.rs crates/app/src/memory/runtime_config.rs
git commit -m "feat: add hydrated memory orchestrator foundation"
```

### Task 4: Route Context Assembly Through The Orchestrator

**Files:**
- Modify: `crates/app/src/conversation/context_engine.rs`
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add tests that assert:

- the default context engine builds context from the orchestrator snapshot
- built-in behavior stays byte-for-byte equivalent for the existing memory
  profiles
- system selection does not change prompt output when `system = "builtin"`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app default_runtime_build_context -- --nocapture`

Expected: FAIL because context assembly still depends on the older direct
memory-window path.

**Step 3: Write minimal implementation**

Refactor `DefaultContextEngine` to consume the typed hydrated-memory snapshot,
but preserve the current prompt ordering and existing profile behavior.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app default_runtime_build_context -- --nocapture`

Run: `cargo test -p loongclaw-app provider:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/context_engine.rs crates/app/src/conversation/runtime.rs crates/app/src/provider/request_message_runtime.rs crates/app/src/conversation/tests.rs
git commit -m "refactor: route context assembly through memory orchestrator"
```

### Task 5: Expand Canonical Persistence To Typed Records

**Files:**
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/conversation/persistence.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add tests that assert:

- provider turns persist typed canonical records
- automatic and explicit ACP routing persist canonical records compatible with
  later derivation
- ACP still bypasses provider-only context-engine lifecycle hooks

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime_automatic_acp_routing_bypasses_context_engine_lifecycle_hooks -- --exact`

Run: `cargo test -p loongclaw-app persist_turn -- --nocapture`

Expected: FAIL because typed canonical record persistence does not exist yet.

**Step 3: Write minimal implementation**

Extend the persistence path so canonical records can carry kind, scope, and
metadata while remaining backward-compatible with the current SQLite baseline.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime_automatic_acp_routing_bypasses_context_engine_lifecycle_hooks -- --exact`

Run: `cargo test -p loongclaw-app persist_turn -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/memory/mod.rs crates/app/src/conversation/persistence.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "feat: add typed canonical memory record persistence"
```

### Task 6: Add Fail-Open Behavior And Built-In Contract Tests

**Files:**
- Modify: `crates/app/src/memory/orchestrator.rs`
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/conversation/tests.rs`
- Test: `crates/app/src/memory/orchestrator.rs`

**Step 1: Write the failing test**

Add tests that assert:

- derivation failure falls back to built-in recent-window behavior
- retrieval failure falls back to built-in recent-window behavior
- strict-mode behavior remains reserved and disabled by default

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app fail_open_memory -- --nocapture`

Expected: FAIL because fail-open policy is not wired into the orchestrator yet.

**Step 3: Write minimal implementation**

Add fail-open policy evaluation and diagnostics so external memory system
problems do not break the baseline chat experience.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app fail_open_memory -- --nocapture`

Run: `cargo test -p loongclaw-app conversation::tests:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/memory/orchestrator.rs crates/app/src/config/tools_memory.rs crates/app/src/conversation/tests.rs
git commit -m "feat: add fail-open memory system policy"
```

### Task 7: Keep Concrete External Adapters Deferred

**Files:**
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/memory/orchestrator.rs`
- Modify: `crates/app/src/memory/system_registry.rs`
- Delete if present: `crates/app/src/memory/adapters/mod.rs`
- Delete if present: `crates/app/src/memory/adapters/<adapter>.rs`

**Step 1: Write the failing test**

Add tests that assert:

- unsupported future ids remain rejected
- the registry stays builtin-only
- fail-open built-in hydration behavior remains intact

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app config::tools_memory::tests::memory_system_rejects_unimplemented_future_variant_ids -- --exact --nocapture`

Run: `cargo test -p loongclaw-app config::tests::memory_system_field_rejects_unimplemented_future_variant -- --exact --nocapture`

Run: `cargo test -p loongclaw-app memory::system_registry::tests::memory_system_registry_stays_builtin_only_until_adapter_lands -- --exact --nocapture`

Expected: FAIL because a concrete experimental adapter is still exposed.

**Step 3: Write minimal implementation**

- remove concrete non-builtin ids from config and registry
- keep the generic memory-system metadata and capability seam
- keep fail-open diagnostics and session-scoped fault injection stability
- defer the first real adapter to a later dedicated track

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app config::tools_memory::tests::memory_system_rejects_unimplemented_future_variant_ids -- --exact --nocapture`

Run: `cargo test -p loongclaw-app config::tests::memory_system_field_rejects_unimplemented_future_variant -- --exact --nocapture`

Run: `cargo test -p loongclaw-app memory::system_registry::tests::memory_system_registry_stays_builtin_only_until_adapter_lands -- --exact --nocapture`

Run: `cargo test -p loongclaw-app fail_open_memory -- --nocapture`

Expected: PASS for builtin-only surface and fail-open stability checks.

**Step 5: Commit**

```bash
git add crates/app/src/config/mod.rs crates/app/src/config/tools_memory.rs crates/app/src/memory/mod.rs crates/app/src/memory/orchestrator.rs crates/app/src/memory/system_registry.rs
git commit -m "refactor: keep memory architecture builtin-only for now"
```

### Task 8: Add Operator-Facing Docs And Runtime Diagnostics

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/roadmap.md`
- Modify: `crates/daemon/src/main.rs`

**Step 1: Write the failing test**

For CLI diagnostics, add tests that assert:

- memory-system listing shows built-in metadata
- diagnostics print selected system and fail-open policy

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon memory_systems -- --nocapture`

Expected: FAIL because the diagnostics and docs are incomplete.

**Step 3: Write minimal implementation**

Document:

- canonical store vs external memory-system boundary
- supported systems and capability differences
- failure policy and fallback behavior

Expose operator diagnostics for selected memory system and capabilities.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon memory_systems -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add README.md README.zh-CN.md docs/roadmap.md crates/daemon/src/main.rs
git commit -m "docs: describe pluggable memory system architecture"
```
