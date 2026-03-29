# Durable Recall Bootstrap Implementation Plan

Date: 2026-03-23
Issue: #469
Depends on: #468
Stack base: `8a5b631b feat(app): flush durable memory before compaction`

## Goal

Add the smallest correct durable-memory read path after `#468` so LoongClaw can
bootstrap advisory durable recall from workspace memory files into runtime
context without jumping into semantic search or new tool surfaces.

## Scope

In scope:

- load advisory durable recall from workspace files under the configured safe
  file root
- support:
  - `MEMORY.md`
  - `memory/MEMORY.md`
  - recent `memory/YYYY-MM-DD.md` daily logs
- inject the resulting material into hydrated runtime memory as advisory durable
  recall
- keep runtime self and resolved runtime identity authoritative
- add deterministic tests and docs

Out of scope:

- embeddings
- sqlite-vec
- FTS5
- query extraction
- `memory_search` / `memory_get`
- identity promotion from durable memory files

## Design Choice

Use the memory hydration path, not the runtime-self system-prompt bootstrap.

Reasoning:

- durable recall is memory context, not runtime self
- this keeps memory, runtime self, and runtime identity in distinct lanes
- provider message assembly already treats hydrated memory entries as explicit
  system or history messages

## Implementation Shape

1. Add a dedicated durable-recall loader in the memory subsystem.
   - collect candidate workspace roots in the same root + nested-workspace style
     as runtime self
   - discover and read:
     - root `MEMORY.md`
     - `memory/MEMORY.md`
     - the newest dated daily logs under `memory/`
   - deduplicate by canonical path
   - bound loaded content deterministically

2. Thread safe workspace-root awareness into memory hydration.
   - hydration should only read durable recall when the caller explicitly passes
     a safe workspace root
   - callers without an explicit safe file root should continue to get no
     durable recall

3. Represent injected durable recall as an explicit memory context entry.
   - prefer a new typed memory entry kind rather than overloading summary or
     profile
   - provider message assembly should render it as a system message

4. Keep the rendered content advisory by construction.
   - add a dedicated durable-recall runtime intro
   - label the block clearly
   - avoid wording that could imply identity authority

## TDD Plan

Write tests before implementation for:

1. Hydration includes durable recall when workspace memory files exist.
2. Hydration skips durable recall when no safe workspace root is provided.
3. Provider message assembly includes the durable recall system block.
4. Identity-looking durable recall content remains advisory and does not replace
   runtime identity.
5. Duplicate or empty files do not produce duplicate prompt entries.

## Files Expected To Change

- `crates/app/src/memory/mod.rs`
- `crates/app/src/memory/orchestrator.rs`
- `crates/app/src/memory/protocol.rs`
- `crates/app/src/memory/` new durable recall helper module
- `crates/app/src/provider/request_message_runtime.rs`
- `crates/app/src/runtime_self.rs` only if shared workspace-root helper reuse is
  cleaner than duplication
- relevant memory/provider tests
- product spec docs if behavior becomes user-visible enough to document

## Validation Plan

- `cargo fmt --all --check`
- `cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings`
- focused failing tests first, then passing targeted tests
- `cargo test -p loongclaw-app --lib`
- if unchanged external blocker remains, note workspace-level validation status
  separately from app-level verification
