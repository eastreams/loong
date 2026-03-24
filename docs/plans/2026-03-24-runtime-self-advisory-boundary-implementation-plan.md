# Runtime-Self Advisory Boundary Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Harden prompt-time authority boundaries so advisory profile, summary, and durable-recall content cannot masquerade as runtime-self or runtime-identity authority.

**Architecture:** Add one small shared advisory-governance helper, apply it to session-profile rendering and shared memory-entry projection, and lock the behavior with red-green regression tests. Keep the existing prompt topology and avoid overlap with `#464` staged hydration internals.

**Tech Stack:** Rust, existing runtime-self and memory projection code, focused unit tests, provider/context-engine parity tests, cargo fmt, clippy, workspace tests.

---

### Task 1: Write the failing advisory-boundary tests

**Files:**
- Modify: `crates/app/src/runtime_identity.rs`
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Add a failing profile test**

Add a test that passes `# Identity` through `render_session_profile_section(...)`
and expects the rendered profile section to keep the text visible while no
longer preserving a raw identity heading.

**Step 2: Add a failing durable-recall projection test**

Add a provider-level test that loads durable recall content containing
runtime-owned headings and expects the projected advisory message to keep the
content visible while demoting those headings.

**Step 3: Add a failing direct-vs-kernel parity test**

Add a conversation-level test that uses an identity-shaped `profile_note` and
expects provider-direct and default-context-engine projection to agree on the
sanitized profile output.

**Step 4: Add a lock test for soul guidance**

Add a test that puts identity-looking content into `SOUL.md` and confirms that
no `Resolved Runtime Identity` section is produced.

**Step 5: Run the targeted tests to confirm red**

Run:

```bash
cargo test -p loongclaw-app render_session_profile_section
cargo test -p loongclaw-app durable_recall
cargo test -p loongclaw-app explicit_builtin_system
```

Expected:
- at least the new profile and durable-recall governance tests fail for the
  missing demotion behavior

### Task 2: Add the shared advisory-governance helper

**Files:**
- Create: `crates/app/src/advisory_prompt.rs`
- Modify: `crates/app/src/lib.rs`

**Step 1: Add governed heading recognition**

Implement a small deterministic helper that:
- detects markdown heading lines
- normalizes heading text
- identifies runtime-owned or identity-like headings

**Step 2: Add deterministic demotion**

Rewrite only governed headings into visible advisory-reference text.

Do not rewrite normal prose.

Do not rewrite non-heading lines.

**Step 3: Add helper-level unit tests**

Add unit tests for:
- runtime-owned heading demotion
- identity heading demotion
- normal advisory text passthrough

### Task 3: Apply governance to session-profile rendering

**Files:**
- Modify: `crates/app/src/runtime_identity.rs`

**Step 1: Sanitize advisory profile content**

Apply the helper to the advisory profile body before wrapping it in
`## Session Profile`.

**Step 2: Keep identity resolution unchanged**

Do not change `resolve_runtime_identity(...)`.

Keep authority sources exactly as they are today.

**Step 3: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app render_session_profile_section
```

Expected:
- profile governance tests pass

### Task 4: Apply governance through one shared memory projection path

**Files:**
- Modify: `crates/app/src/provider/request_message_runtime.rs`
- Modify: `crates/app/src/provider/mod.rs`
- Modify: `crates/app/src/conversation/context_engine.rs`

**Step 1: Add one shared memory-entry projection helper**

Move the duplicated advisory-entry projection match into one provider-shared
helper.

**Step 2: Sanitize advisory entries during prompt projection**

For `Profile`, `Summary`, and `RetrievedMemory`:
- sanitize the content before building the prompt message

For `Turn`:
- keep the existing history-message path unchanged

**Step 3: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app durable_recall
cargo test -p loongclaw-app explicit_builtin_system
```

Expected:
- provider and context-engine governance tests pass

### Task 5: Document the boundary

**Files:**
- Modify: `docs/product-specs/runtime-self-continuity.md`

**Step 1: Update the spec**

Document that advisory profile, summary, and durable-recall projection demote
runtime-owned or identity-like headings rather than letting them re-enter the
prompt as authoritative sections.

### Task 6: Run full verification

**Files:**
- Verify only

**Step 1: Format**

```bash
cargo fmt --all
cargo fmt --all --check
```

**Step 2: Run focused tests**

```bash
cargo test -p loongclaw-app render_session_profile_section
cargo test -p loongclaw-app durable_recall
cargo test -p loongclaw-app explicit_builtin_system
```

**Step 3: Run touched-surface lint**

```bash
cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings
```

**Step 4: Run full workspace tests**

```bash
cargo test --workspace --locked
```

Expected:
- touched-surface checks pass
- full workspace tests pass

### Task 7: Prepare clean delivery

**Files:**
- Modify only the files touched by this plan

**Step 1: Inspect scope**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

**Step 2: Commit**

```bash
git add docs/plans/2026-03-24-runtime-self-advisory-boundary-design.md
git add docs/plans/2026-03-24-runtime-self-advisory-boundary-implementation-plan.md
git add crates/app/src/advisory_prompt.rs
git add crates/app/src/lib.rs
git add crates/app/src/runtime_identity.rs
git add crates/app/src/provider/request_message_runtime.rs
git add crates/app/src/provider/mod.rs
git add crates/app/src/conversation/context_engine.rs
git add crates/app/src/conversation/tests.rs
git add docs/product-specs/runtime-self-continuity.md
git commit -m "fix(app): harden advisory runtime-self prompt boundaries"
```
