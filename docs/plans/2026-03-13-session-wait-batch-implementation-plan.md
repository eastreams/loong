# Session Wait Batch Implementation Plan

**Goal:** deepen the existing `session_wait` tool so operators can wait on multiple visible
sessions through one request, without adding a new top-level tool and without regressing the
single-target contract.

**Architecture:** keep `tools/mod.rs` as the async app-tool wrapper for `session_wait`, but move
the actual wait orchestration into `tools/session.rs` beside the rest of the session tool logic.
Batch mode reuses the existing inspection and wait payloads per target and wraps them in the same
batch-result envelope pattern already used by `session_status`, `session_cancel`, and
`session_recover`.

## Task 1: Extend the provider schema

Update the `session_wait` catalog definition so it accepts:

- `session_id`
- `session_ids`

Keep `after_id` and `timeout_ms` unchanged.

Acceptance criteria:

- provider schema exposes `session_ids`
- schema uses `oneOf` to require exactly one of `session_id` or `session_ids`

## Task 2: Add failing behavior coverage first

Add tests for:

- provider schema stability for batch `session_wait`
- mixed batch outcomes:
  - one visible terminal session
  - one visible non-terminal session that times out
  - one hidden session

Acceptance criteria:

- tests fail before implementation for the expected reasons:
  - schema missing `session_ids`
  - runtime rejects `payload.session_ids`

## Task 3: Move wait semantics into `tools/session.rs`

Implement:

- shared request parsing for single or batch targets
- preserved single-target loop semantics
- batch wait loop with:
  - shared timeout window
  - per-target visibility checks
  - early completion for terminal sessions
  - timeout finalization for remaining sessions
  - request-order-preserving results

Acceptance criteria:

- single-target callers still get the legacy wait payload and top-level status
- batch callers get a structured batch response with `ok`, `timeout`, and `skipped_not_visible`
- `after_id` continues to populate per-target `events` and `next_after_id`

## Task 4: Document the surface

Update:

- `docs/product-specs/index.md`
- `docs/roadmap.md`
- this design and implementation plan pair

Acceptance criteria:

- product spec captures batch wait as part of the session operator surface
- roadmap notes that wait joins the existing batch status/remediation family

## Verification

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app provider_tool_definitions_are_stable_and_complete -- --nocapture --test-threads=1
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app session_wait_ -- --nocapture --test-threads=1
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test --workspace --all-features -- --test-threads=1
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all --check
git diff --check
```

