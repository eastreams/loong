# Session Tool List Pagination Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `offset`-based pagination to `sessions_list` without changing existing visibility or
sorting semantics.

**Architecture:** Keep pagination inside the app-tool layer. Parse one new optional `offset`
parameter, apply it after the current filter pipeline and before truncation, and surface a minimal
response contract with `has_more`.

**Tech Stack:** Rust, serde_json, existing app-tool tests

---

## Task 1: Add the failing payload and tool behavior tests

**Files:**
- Modify: `crates/app/src/tools/payload.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Write the failing payload test**

Add tests for a new `optional_payload_offset()` helper:

- missing offset returns `0`
- positive offset returns the same value
- invalid or negative offset returns `0`

**Step 2: Write the failing session tool test**

Add a `sessions_list` test that:

- seeds one root plus three visible child sessions
- fixes their `updated_at` ordering deterministically
- calls `sessions_list` with `limit=2` and `offset=1`
- expects the second and third sessions in sort order
- expects `matched_count=4`, `returned_count=2`, and `has_more=true`

**Step 3: Run the focused tests and verify red**

Run:

```bash
cargo test -p loongclaw-app optional_payload_offset --locked
cargo test -p loongclaw-app sessions_list_applies_offset_pagination --locked
```

Expected: failing because `offset` is not implemented yet.

## Task 2: Implement the pagination helper and request parsing

**Files:**
- Modify: `crates/app/src/tools/payload.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Add `optional_payload_offset()`**

Return a non-negative `usize` with `0` as the default.

**Step 2: Extend `SessionsListRequest`**

Add an `offset` field and parse it in `parse_sessions_list_request()`.

**Step 3: Re-run the focused tests**

Run:

```bash
cargo test -p loongclaw-app optional_payload_offset --locked
cargo test -p loongclaw-app sessions_list_applies_offset_pagination --locked
```

Expected: green.

## Task 3: Wire pagination metadata into the `sessions_list` response contract

**Files:**
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Apply `offset` after filtering**

Compute:

- `matched_count`
- effective page slice
- `returned_count`
- `has_more`

**Step 2: Surface pagination metadata**

Add `offset` to the `filters` payload and add `has_more` to the top-level response.

**Step 3: Keep existing behavior unchanged when `offset` is absent**

Confirm legacy calls still return the same first page.

## Task 4: Update tool contract metadata

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`

**Step 1: Add `offset` to `sessions_list_definition()`**

Describe it as the number of matching visible sessions to skip before applying `limit`.

**Step 2: Update compact hint metadata**

Add `offset` to the `sessions_list` argument hint and parameter type list.

**Step 3: Add or update a contract-focused test if needed**

Keep coverage local to the existing catalog tests.

## Task 5: Run focused and broader verification

**Files:**
- Modify: none unless verification exposes a necessary fix

**Step 1: Run the focused app-tool tests**

Run:

```bash
cargo test -p loongclaw-app optional_payload_offset --locked
cargo test -p loongclaw-app sessions_list --locked
```

**Step 2: Run relevant conversation/tool contract coverage**

Run:

```bash
cargo test -p loongclaw-app sessions_list --locked
cargo test -p loongclaw-app tool_catalog --locked
```

**Step 3: Run formatting, lint, and full workspace tests**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
```

Expected: all green.

## Task 6: Prepare clean delivery

**Files:**
- Modify: GitHub artifacts through `gh`, not repository files

**Step 1: Inspect isolated changes**

Run:

```bash
git status --short
git diff -- crates/app/src/tools/payload.rs crates/app/src/tools/session.rs crates/app/src/tools/catalog.rs docs/plans/2026-04-01-session-tool-list-pagination-design.md docs/plans/2026-04-01-session-tool-list-pagination-implementation-plan.md
```

**Step 2: Commit with a scoped message**

Run:

```bash
git add crates/app/src/tools/payload.rs crates/app/src/tools/session.rs crates/app/src/tools/catalog.rs docs/plans/2026-04-01-session-tool-list-pagination-design.md docs/plans/2026-04-01-session-tool-list-pagination-implementation-plan.md
git commit -m "Add pagination to session tool listings"
```

**Step 3: Create linked issue and PR**

Use the repository templates, English GitHub copy, and an explicit closing clause in the PR body.
