# Memory Search Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a truthful `memory_search` app tool that searches visible transcript turns and returns structured snippet matches without claiming semantic memory.

**Architecture:** Implement transcript search on top of the existing SQLite `turns` store, reuse session visibility and legacy fallback rules from the session repository, and expose `memory_search` as a new app-layer tool with a narrow provider schema. Land the feature through TDD in small slices: repository-free query helper first, then app-tool surface and scope resolution, then product docs and full verification.

**Tech Stack:** Rust, `rusqlite`, existing LoongClaw app tool catalog and dispatcher, SQLite-backed session repository, Cargo tests.

---

### Task 1: Add the failing transcript-search memory tests

**Files:**
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/memory/sqlite.rs`

**Step 1: Write the failing tests**

Add focused unit tests in `crates/app/src/memory/mod.rs` or `crates/app/src/memory/sqlite.rs` for:

- `search_transcript_direct_returns_recent_matching_turns`
- `search_transcript_direct_excludes_non_matching_turns`
- `search_transcript_direct_clamps_limit_and_excerpt`

The tests should:

- create an isolated SQLite path with `MemoryRuntimeConfig`
- append several turns across one or more sessions
- search for a phrase that appears in multiple rows
- assert that results include `turn_id`, `session_id`, `role`, `ts`, and a clipped snippet
- assert newest-first ordering
- assert a no-match query returns an empty list

**Step 2: Run test to verify it fails**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app search_transcript_direct -- --nocapture --test-threads=1
```

Expected:

- compile or test failure because transcript-search helpers and result structs do not exist yet

**Step 3: Write minimal implementation**

Implement the minimum SQLite search support in `crates/app/src/memory/sqlite.rs`:

- add a search result struct that includes `turn_id`, `session_id`, `role`, `content`, and `ts`
- add a helper that executes a substring-style query against `turns.content`
- add safe clamping for limit and excerpt sizes
- add a direct helper callable from tests

Keep the implementation narrow:

- search only `turns`
- use recent-first ordering
- do not add FTS, extra indexes, or semantic-ranking behavior

**Step 4: Run test to verify it passes**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app search_transcript_direct -- --nocapture --test-threads=1
```

Expected:

- the new transcript-search tests pass

**Step 5: Commit**

```bash
git add crates/app/src/memory/mod.rs crates/app/src/memory/sqlite.rs
git commit -m "feat(app): add transcript memory search helpers"
```

### Task 2: Add the failing app-tool behavior tests for `memory_search`

**Files:**
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Create: `crates/app/src/tools/memory.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing tests**

Add focused app-tool tests alongside the existing session-tool coverage, preferably in
`crates/app/src/tools/memory.rs` if that module owns its own tests.

Add tests for:

- `memory_search_searches_visible_scope_by_default`
- `memory_search_rejects_invisible_single_target`
- `memory_search_batch_reports_skipped_targets`
- `memory_search_includes_archived_visible_sessions`
- `memory_search_supports_legacy_current_session_transcript`
- `memory_search_does_not_return_session_events`

Test fixtures should:

- build isolated `MemoryRuntimeConfig`
- create root and delegate-child sessions with `SessionRepository`
- archive one finished visible session to prove archived transcripts still search
- append transcript turns and control-plane events separately
- execute the tool through `execute_app_tool_with_config`

**Step 2: Run test to verify it fails**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app memory_search_ -- --nocapture --test-threads=1
```

Expected:

- compile or test failure because the tool is not cataloged or implemented yet

**Step 3: Write minimal implementation**

Implement the app-layer tool:

- add `memory_search` to `crates/app/src/tools/catalog.rs`
- expose a provider schema with `query`, `session_id`, `session_ids`, `limit`, and `excerpt_chars`
- route `memory_search` from `crates/app/src/tools/mod.rs`
- create `crates/app/src/tools/memory.rs` to own:
  - request parsing
  - visible-scope target resolution
  - single-target visibility checks
  - batch skipped-target accounting
  - snippet shaping and top-level response payload

Reuse existing session visibility and legacy-fallback behavior rather than re-inventing it.

**Step 4: Run test to verify it passes**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app memory_search_ -- --nocapture --test-threads=1
```

Expected:

- the new `memory_search` app-tool tests pass

**Step 5: Commit**

```bash
git add crates/app/src/tools/mod.rs crates/app/src/tools/catalog.rs crates/app/src/tools/memory.rs crates/app/src/tools/session.rs
git commit -m "feat(app): add memory search tool surface"
```

### Task 3: Add provider-view and runtime registration coverage

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add or extend tests to prove:

- `memory_search` appears in the runtime tool registry when session tools are enabled
- provider tool definitions include the new function schema
- canonical tool-name resolution works for `memory_search`

If existing tests already cover adjacent registry behavior, extend them with `memory_search`
assertions instead of creating redundant new fixtures.

**Step 2: Run test to verify it fails**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app tool_registry -- --nocapture --test-threads=1
```

Expected:

- failure because the registry/provider surface does not yet advertise `memory_search` completely

**Step 3: Write minimal implementation**

Adjust the tool view and registration paths so `memory_search` is treated as a truthful runtime app
tool in the same enablement family as other session-scoped app tools.

**Step 4: Run test to verify it passes**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app tool_registry -- --nocapture --test-threads=1
```

Expected:

- the registry/provider assertions pass

**Step 5: Commit**

```bash
git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs
git commit -m "feat(app): advertise memory search runtime tool"
```

### Task 4: Update product docs and acceptance criteria

**Files:**
- Modify: `docs/product-specs/index.md`
- Modify: `docs/roadmap.md`

**Step 1: Write the doc changes**

Update product-facing docs to reflect the new tool:

- add `memory_search` to the accepted root tool surface
- document the truthful phase-one scope: transcript search over visible sessions
- note that semantic memory, FTS, and vector retrieval remain out of scope

Keep the wording aligned with the design doc rather than promising future capabilities as if they
already ship.

**Step 2: Review the doc diff**

Run:

```bash
git diff -- docs/product-specs/index.md docs/roadmap.md
```

Expected:

- only the documented `memory_search` surface and roadmap wording changed

**Step 3: Commit**

```bash
git add docs/product-specs/index.md docs/roadmap.md
git commit -m "docs: describe memory search tool surface"
```

### Task 5: Full verification and cleanup

**Files:**
- Modify if needed: any files touched above

**Step 1: Run focused package tests**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app memory_search_ search_transcript_direct tool_registry -- --nocapture --test-threads=1
```

Expected:

- focused new coverage passes

**Step 2: Run the full app package tests**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app -- --nocapture --test-threads=1
```

Expected:

- all `loongclaw-app` tests pass

**Step 3: Run formatting**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all
```

Expected:

- no formatting errors

**Step 4: Re-run the full app package tests after formatting**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app -- --nocapture --test-threads=1
```

Expected:

- all `loongclaw-app` tests still pass after formatting

**Step 5: Compile daemon tests without running**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-daemon --no-run
```

Expected:

- daemon targets compile successfully, proving the app-tool surface change does not break daemon integration

**Step 6: Inspect the final branch state**

Run:

```bash
git status --short
git log --oneline -6
```

Expected:

- clean worktree
- small, isolated commits for helpers, tool surface, docs, and formatting if needed

**Step 7: Commit any final formatting-only adjustments**

```bash
git add -A
git commit -m "style(rust): format memory search changes"
```

Only do this step if formatting changed tracked files and those changes are not already included in
an earlier commit.
