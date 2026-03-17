# Alpha-Test Internal Runtime Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn the `alpha-test` internal runtime hardening direction into a clean execution package:
one design doc, one implementation plan, one GitHub backlog document, one roadmap update, and a
submission order for the next code streams.

**Architecture:** Reuse the repository's existing kernelization, audit, provider-runtime, and ACP
design work instead of inventing a parallel planning stack. Keep this slice documentation-only, but
make each next code stream specific enough that an engineer can pick it up without redoing the
research.

**Tech Stack:** Markdown docs, repository planning conventions, GitHub issue/PR templates, `rg`,
`cargo test`, git diff

---

### Task 1: Lock the planning package in repo docs

**Files:**
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-design.md`
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-implementation-plan.md`
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md`
- Modify: `docs/ROADMAP.md`

**Step 1: Re-read the key evidence seams**

Run:

```bash
rg -n "ConversationRuntimeBinding|ProviderRuntimeBinding|InMemoryAuditSink|AcpSessionManager|ACPX_BACKEND_ID|Current Priority Order" \
  crates/app crates/kernel docs/ROADMAP.md docs/SECURITY.md
```

Expected: the binding, audit, ACP, and roadmap seams are fully enumerated before writing.

**Step 2: Confirm the planning files exist**

Run:

```bash
ls \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-design.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-implementation-plan.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md
```

Expected: all three planning files exist.

### Task 2: Align the repository roadmap with the same priority order

**Files:**
- Modify: `docs/ROADMAP.md`

**Step 1: Update the roadmap timestamp and near-term priorities**

Refresh `Last updated` and replace the old current-priority list with the internal runtime-hardening
order:

1. kernel-first runtime closure
2. persistent audit sink and query baseline
3. ACP control-plane hardening
4. cross-lane execution tiers
5. first-party workflow packs on hardened primitives

**Step 2: Add the missing discussion items**

Add short discussion entries for:

1. governed/direct runtime closure
2. ACP control-plane hardening
3. execution security tiers
4. first-party workflow packs

Keep the prose short and execution-focused.

**Step 3: Review the scoped roadmap diff**

Run:

```bash
git diff -- docs/ROADMAP.md
```

Expected: only the intended priority and discussion updates are present.

### Task 3: Prepare GitHub delivery for the planning package

**Files:**
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md`

**Step 1: Draft the docs issue**

Write one documentation-improvement issue draft for the current planning package so the docs PR has
a clean issue to close.

**Step 2: Draft the umbrella feature issue**

Write one umbrella feature issue for the runtime hardening program itself.

**Step 3: Reuse or replace tracking issues correctly**

Document in the backlog that:

1. issue `#172` should be reused for persistent audit because it is already open
2. issue `#45` should not be reused because it is closed

**Step 4: Draft the child issues and the docs PR body**

Include:

1. one new issue for kernel-first runtime closure
2. one new issue for ACP control-plane hardening
3. one new issue for execution security tiers
4. one deferred issue for workflow packs
5. one PR body that closes the docs issue and references the umbrella/runtime issues

### Task 4: Define Stream 1 as the next code track

**Files:**
- Reference: `docs/plans/2026-03-15-conversation-runtime-binding-design.md`
- Reference: `docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md`
- Reference: `docs/plans/2026-03-16-governed-runtime-path-hardening-design.md`
- Reference: `docs/plans/2026-03-15-provider-binding-normalization-design.md`
- Modify next: `crates/app/src/conversation/runtime_binding.rs`
- Modify next: `crates/app/src/provider/runtime_binding.rs`
- Modify next: `crates/app/src/conversation/turn_coordinator.rs`
- Modify next: `crates/app/src/conversation/session_history.rs`
- Modify next: `docs/SECURITY.md`

**Step 1: Re-scope the remaining direct lanes**

Run:

```bash
rg -n "ConversationRuntimeBinding::direct|ProviderRuntimeBinding::direct|no_kernel_context|Direct" \
  crates/app/src
```

Expected: direct-mode compatibility seams are enumerated before opening the issue.

**Step 2: Prepare the first code slice around one bounded seam**

The first Stream 1 implementation should be a narrow bounded slice such as:

1. one more governed/direct closure in conversation history or turn orchestration
2. one provider-runtime governed seam
3. one docs truthfulness refresh

Do not open Stream 1 as a repo-wide mega-refactor.

### Task 5: Treat Stream 2 as an immediate follow-on using the existing open issue

**Files:**
- Reference: `docs/plans/2026-03-15-persistent-audit-sink-design.md`
- Reference: `docs/plans/2026-03-15-persistent-audit-sink-implementation-plan.md`
- Modify next: `crates/kernel/src/audit.rs`
- Modify next: `crates/app/src/context.rs`
- Modify next: `crates/spec/src/kernel_bootstrap.rs`
- Modify next: `docs/SECURITY.md`
- Modify next: `docs/RELIABILITY.md`

**Step 1: Verify issue reuse is explicit**

The backlog document must say Stream 2 reuses issue `#172` and does not open a duplicate.

**Step 2: Keep the first durable-audit slice query-light**

Land the durable sink first. Query ergonomics may stay minimal in the first slice as long as
operators can inspect and filter the retained evidence.

### Task 6: Scope Stream 3 as decomposition and recovery, not capability expansion

**Files:**
- Reference: `docs/design-docs/acp-acpx-preembed.md`
- Modify next: `crates/app/src/acp/manager.rs`
- Modify next: `crates/app/src/acp/acpx.rs`
- Modify next: `crates/app/src/acp/backend.rs`
- Modify next: `crates/app/src/acp/store.rs`
- Modify next: `crates/app/src/conversation/turn_coordinator.rs`

**Step 1: Enumerate ACP ownership boundaries**

Run:

```bash
rg -n "pub async fn|fn " crates/app/src/acp/manager.rs crates/app/src/acp/acpx.rs
```

Expected: the session lifecycle, actor queue, status, cancellation, and transport responsibilities
are visible before decomposition starts.

**Step 2: Open the issue around recovery and decomposition**

The issue must emphasize:

1. smaller ownership boundaries
2. stuck-turn recovery and cancel/close repair
3. better observability
4. no new ACP surface-area inflation in the first slice

### Task 7: Scope Stream 4 as one shared execution-tier model

**Files:**
- Modify next: `docs/SECURITY.md`
- Modify next: `docs/ROADMAP.md`
- Modify next: `crates/kernel/src/policy.rs`
- Modify next: bridge execution surfaces in `crates/spec`
- Modify next: browser/runtime surfaces in `crates/daemon`

**Step 1: Enumerate the current lane-specific security words**

Run:

```bash
rg -n "restricted|balanced|trusted|sandbox|wasm|browser|process_stdio|http_json" \
  crates docs/SECURITY.md docs/ROADMAP.md
```

Expected: the current lane vocabulary and drift points are visible.

**Step 2: Keep the first execution-tier issue architecture-first**

Do not start with a giant sandbox rewrite. Start with one shared tier model and one lane at a
time.

### Task 8: Defer Stream 5 until Streams 1-4 are underway

**Files:**
- Create later: pack-specific docs, manifests, and product specs

**Step 1: Keep workflow packs as a deferred issue**

The backlog should explicitly label workflow packs as dependent on the hardened runtime base.

**Step 2: Do not let workflow-pack enthusiasm reshuffle the runtime priorities**

If a later PR adds a workflow pack before Streams 1-4 move, the reviewer should treat that as a
priority-order violation unless it is purely exploratory.

### Task 9: Run verification and prepare local delivery

**Files:**
- Modify: `docs/ROADMAP.md`
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-design.md`
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-implementation-plan.md`
- Create: `docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md`

**Step 1: Run workspace tests for clean baseline confidence**

Run:

```bash
cargo test --workspace --locked
```

Expected: PASS.

**Step 2: Review the scoped docs diff**

Run:

```bash
git diff -- \
  docs/ROADMAP.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-design.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-implementation-plan.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md
```

Expected: only the planning package is present.

**Step 3: Commit the planning package**

Run:

```bash
git add \
  docs/ROADMAP.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-design.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-implementation-plan.md \
  docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md
git commit -m "docs: add alpha-test internal runtime hardening roadmap"
```

Expected: one clean docs-only commit is created on the worktree branch.
