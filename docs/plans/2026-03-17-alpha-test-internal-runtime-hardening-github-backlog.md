# Alpha-Test Internal Runtime Hardening GitHub Backlog

This document prepares the current planning package and the next runtime-hardening streams for
clean GitHub issue/PR submission.

## Submission Order

1. Open the docs issue below.
2. Open the umbrella feature issue below.
3. Open the new Stream 1 issue below.
4. Reuse open issue `#172` for Stream 2.
5. Open the Stream 3 and Stream 4 issues below.
6. Open the docs PR using the draft at the end of this document.
7. Leave the workflow-pack issue as deferred unless Streams 1-4 are already underway.

## Existing Tracking To Reuse Or Avoid

### Reuse

- Issue `#172` is open:
  `[Feature] Add persistent kernel audit sink and fanout boundary`

### Do Not Reuse

- Issue `#45` is closed:
  `Policy Unification: five parallel security decision paths bypass the kernel policy chain`

The remaining kernel-closure work is real, but it needs a new bounded follow-up issue instead of
reopening a closed broad unification track.

## Docs Issue Draft

Route: `Documentation Improvement`

Suggested title:

```text
[Docs]: Document the alpha-test internal runtime hardening roadmap and GitHub backlog
```

Suggested body:

```text
Area
Docs / contributor workflow

Summary
The repository now has enough internal design momentum that the next runtime-hardening direction
should be captured explicitly in-repo instead of remaining spread across separate plan documents
and chat context.

Current gap
The branch already contains strong design work around conversation runtime bindings, governed
runtime path hardening, persistent audit, provider runtime decomposition, and ACP pre-embed
architecture. However, there is no single planning package that:
- explains the next internal priority order
- distinguishes which existing issues should be reused versus replaced
- prepares GitHub-ready issue and PR copy for the next execution streams

Proposed improvement
Add a documentation package that includes:
- one design doc for the internal runtime hardening direction
- one implementation plan for turning that direction into execution tracks
- one GitHub backlog document with issue/PR drafts
- one roadmap update so the same priority order is visible from docs/ROADMAP.md

Evidence / links
- docs/plans/2026-03-16-governed-runtime-path-hardening-design.md
- docs/plans/2026-03-15-persistent-audit-sink-design.md
- docs/design-docs/acp-acpx-preembed.md
- docs/ROADMAP.md

Impact
Maintainers and contributors can reason about the next internal priorities without reconstructing
them from multiple plan files or chat history, and the next issue/PR intake becomes cleaner.
```

## Umbrella Feature Issue Draft

Route: `Feature Request`

Suggested title:

```text
[Feature]: Establish the alpha-test internal runtime hardening program
```

Suggested labels:

```text
enhancement
triage
area: kernel
```

Suggested body:

```text
Area
Kernel / policy / approvals

Problem statement
alpha-test has a strong kernel-first architecture story, but the next internal priorities are
still fragmented across several design docs and partially-implemented runtime seams. The repository
needs one explicit program that clarifies what should land next and in what order.

Proposed solution
Adopt an internal runtime hardening program with the following priority order:
- Stream 1: kernel-first runtime closure and direct-path retirement
- Stream 2: persistent audit sink and query baseline
- Stream 3: ACP control-plane hardening and recovery
- Stream 4: cross-lane execution security tiers
- Stream 5: first-party workflow packs after the runtime base is harder

Non-goals / out of scope
- README or external onboarding refresh
- broad product-surface expansion before the runtime base is harder
- reopening already-closed broad policy issues without a narrower bounded scope

Alternatives considered
- Product-surface-first expansion: rejected because it spends complexity before runtime truth is
  stronger.
- One-off hardening slices with no umbrella: rejected because it leaves priority and issue intake
  fragmented.

Acceptance criteria
- The program priority order is documented in-repo and reflected in docs/ROADMAP.md
- A new Stream 1 issue exists for kernel-first closure follow-up
- Existing open issue #172 is explicitly reused for persistent audit
- New issues exist for ACP hardening and execution tiers
- Workflow packs remain explicitly deferred until the runtime base is harder

Policy / security / breaking sensitivity
Yes, this touches policy, security, or potentially breaking behavior

Rollout / rollback notes
Land this as documentation and issue-intake alignment first. Execute each stream as a separate PR
series with normal CI parity and bounded reviewable slices.

Impact
Affected users: maintainers and internal contributors
Frequency: every near-term architecture and runtime planning decision
Consequence if missing: duplicated issues, drift between roadmap and code-level plans, and weaker
execution discipline on alpha-test
```

## Stream 1 Issue Draft

Route: `Feature Request`

Suggested title:

```text
[Feature]: Retire shadow direct runtime paths behind kernel-bound compatibility seams
```

Suggested labels:

```text
enhancement
triage
area: conversation
```

Suggested body:

```text
Area
Conversation / session runtime

Problem statement
alpha-test now makes conversation and provider governance explicit through
ConversationRuntimeBinding and ProviderRuntimeBinding, but the runtime still carries intentional
Direct compatibility paths deeper than the long-term architecture wants. This keeps a gap between
the kernel-first design language and the actual governed-versus-direct execution story.

Proposed solution
Run the next bounded kernel-closure stream that:
- pushes Direct back toward ingress, explicit compatibility wrappers, and tests
- makes governed reads and governed side effects fail closed instead of silently re-entering direct
  paths
- refreshes security/architecture docs so they accurately describe the remaining compatibility seams

Non-goals / out of scope
- repo-wide kernelization in one PR
- ACP redesign
- durable audit implementation
- workflow-pack or product-surface expansion

Alternatives considered
- Reuse closed issue #45: rejected because the remaining work is narrower and should not reopen a
  closed broad policy-unification issue.
- Add more telemetry without changing behavior: rejected because it improves observability without
  improving architecture truthfulness.

Acceptance criteria
- A bounded first slice closes one more governed/direct drift seam
- Direct behavior remains only where it is explicitly intended
- Kernel-bound failure cases fail closed with tests
- docs/SECURITY.md and related design docs stop over- or under-claiming the runtime contract

Policy / security / breaking sensitivity
Yes, this touches policy, security, or potentially breaking behavior

Rollout / rollback notes
Execute as reviewable slices over specific conversation/provider seams. Avoid a single giant
cross-runtime patch.

Impact
Affected users: maintainers, runtime authors, and operators relying on kernel-first guarantees
Frequency: every governed conversation or provider execution path
Consequence if missing: continued architecture drift and harder future runtime hardening
```

## Stream 2 Tracking Note

Do not open a new issue. Reuse open issue `#172`:

```text
[Feature] Add persistent kernel audit sink and fanout boundary
```

Optional refresh comment to add on `#172`:

```text
This issue is now part of the alpha-test internal runtime hardening program and remains the
authoritative Stream 2 track. Follow-up planning now treats durable audit as the second priority
after the new kernel-closure stream and before ACP control-plane hardening.
```

## Stream 3 Issue Draft

Route: `Feature Request`

Suggested title:

```text
[Feature]: Decompose the ACP control plane for recovery, observability, and lower merge risk
```

Suggested labels:

```text
enhancement
triage
area: acp
```

Suggested body:

```text
Area
ACP control plane

Problem statement
The ACP architecture is now strategically important and directionally correct, but too much control
plane behavior still lives in a few large hotspots such as crates/app/src/acp/manager.rs and
crates/app/src/acp/acpx.rs. That raises merge risk, recovery risk, and observability complexity.

Proposed solution
Run an ACP hardening stream that:
- decomposes manager responsibilities into smaller ownership boundaries
- separates ACPX process/transport concerns from control-plane semantics
- adds explicit stuck-turn recovery, cancel/close repair, and better status observability
- keeps runtime-event handling on a path toward durable evidence instead of process-local-only state

Non-goals / out of scope
- broad new ACP feature expansion
- moving ACP into provider or context-engine architecture
- redesigning all ACP backends in one pass

Alternatives considered
- Keep adding features inside the current large files: rejected because it compounds control-plane
  debt.
- Collapse ACP back into other runtimes: rejected because the current architecture direction is
  correct and should be hardened, not undone.

Acceptance criteria
- ACP manager/backend responsibilities are visibly smaller and easier to review
- stuck-turn and repair semantics are explicit and tested
- observability no longer depends on reconstructing backend-local behavior from large files
- the first slice lands without broad ACP surface-area inflation

Policy / security / breaking sensitivity
Yes, this touches policy, security, or potentially breaking behavior

Rollout / rollback notes
Split by ownership boundary, not by random line ranges. Preserve current ACP behavior first, then
tighten recovery and observability.

Impact
Affected users: maintainers and operators using ACP or ACPX-backed flows
Frequency: every ACP session lifecycle and routed turn
Consequence if missing: continued control-plane hotspot growth and harder future ACP evolution
```

## Stream 4 Issue Draft

Route: `Feature Request`

Suggested title:

```text
[Feature]: Introduce shared execution security tiers across process, browser, and WASM lanes
```

Suggested labels:

```text
enhancement
triage
area: kernel
```

Suggested body:

```text
Area
Kernel / policy / approvals

Problem statement
The roadmap already points toward sandbox profile tiers, but the repository still needs one shared
execution-tier model across process, browser, and WASM-style lanes. Without that, each lane risks
growing its own security vocabulary and evidence semantics.

Proposed solution
Define and implement one shared execution-tier contract such as restricted/balanced/trusted across
the main runtime lanes, with aligned policy defaults, runtime evidence, and documentation.

Non-goals / out of scope
- one giant sandbox rewrite
- per-lane feature expansion unrelated to the shared tier model
- replacing existing bounded runtime checks that are already correct

Alternatives considered
- Keep per-lane security semantics independent: rejected because it increases drift and weakens the
  operator model.
- Start with browser-only policy work: rejected because the contract should be shared before lane-
  specific polish grows further.

Acceptance criteria
- one shared execution-tier vocabulary exists across the main runtime lanes
- policy defaults and runtime evidence align with that vocabulary
- at least one lane lands on the new tier model without breaking the others
- docs/SECURITY.md and docs/ROADMAP.md reflect the shared contract

Policy / security / breaking sensitivity
Yes, this touches policy, security, or potentially breaking behavior

Rollout / rollback notes
Land the shared contract first, then migrate one lane at a time with bounded regressions.

Impact
Affected users: maintainers, runtime authors, and operators
Frequency: every high-risk execution path
Consequence if missing: policy drift and weaker cross-lane operator understanding
```

## Stream 5 Deferred Issue Draft

Route: `Feature Request`

Suggested title:

```text
[Feature]: Ship first-party workflow packs on the hardened alpha-test runtime base
```

Suggested labels:

```text
enhancement
triage
area: tools
```

Suggested body:

```text
Area
Tools

Problem statement
LoongClaw needs a small set of first-party workflow packs that prove the value of the kernel and
make the productization story concrete, but shipping them before the runtime base is harder would
borrow against control-plane and evidence debt.

Proposed solution
After Streams 1-4 are underway, ship 2-3 first-party workflow packs built on the hardened runtime
base, such as:
- release/review operator workflows
- issue triage and maintenance workflows
- channel or customer-support workflows

Non-goals / out of scope
- moving this work ahead of the runtime-hardening streams
- pack proliferation without strong runtime primitives underneath

Alternatives considered
- product-surface-first acceleration: rejected because the runtime base should harden first

Acceptance criteria
- workflow packs are explicitly treated as dependent on the hardened runtime base
- the first pack set demonstrates kernel value without adding ad hoc runtime exceptions

Policy / security / breaking sensitivity
Not sure

Rollout / rollback notes
Open only after Streams 1-4 are underway or when the maintainers intentionally choose a
productization checkpoint.

Impact
Affected users: future operators and downstream pack authors
Frequency: depends on later productization cadence
Consequence if missing: weaker proof of value after the runtime base is hardened
```

## Docs PR Draft

Suggested title:

```text
docs: add alpha-test internal runtime hardening roadmap
```

Suggested body:

````markdown
## Summary

- Problem:
  The next internal runtime priorities on `alpha-test` were spread across multiple design docs and
  were not yet packaged into a single issue/PR-ready planning bundle.
- Why it matters:
  Maintainers need one clean source of truth for the next internal hardening order, issue reuse,
  and GitHub intake flow.
- What changed:
  Added a design doc, an implementation plan, a GitHub backlog document, and a roadmap update for
  the alpha-test internal runtime hardening program.
- What did not change (scope boundary):
  No runtime behavior, code paths, CI rules, or external README flows changed in this PR.

## Linked Issues

- Closes #<docs-issue-id>
- Related #<umbrella-runtime-hardening-issue-id>
- Related #172

## Change Type

- [ ] Bug fix
- [ ] Feature
- [ ] Refactor
- [x] Documentation
- [ ] Security hardening
- [ ] CI / workflow / release

## Touched Areas

- [ ] Kernel / policy / approvals
- [ ] Contracts / protocol / spec
- [ ] Daemon / CLI / install
- [ ] Providers / routing
- [ ] Tools
- [ ] Browser automation
- [ ] Channels / integrations
- [ ] ACP / conversation / session runtime
- [ ] Memory / context assembly
- [ ] Config / migration / onboarding
- [x] Docs / contributor workflow
- [ ] CI / release / workflows

## Risk Track

- [x] Track A (routine / low-risk)
- [ ] Track B (higher-risk / policy-impacting)

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [x] `cargo test --workspace --locked`
- [ ] `cargo test --workspace --all-features --locked`
- [x] Relevant architecture / dep-graph / docs checks for touched areas
- [ ] Additional scenario, benchmark, or manual checks when behavior changed

Commands and evidence:

```text
cargo test --workspace --locked
git diff -- docs/ROADMAP.md docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-design.md docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-implementation-plan.md docs/plans/2026-03-17-alpha-test-internal-runtime-hardening-github-backlog.md
```

## User-visible / Operator-visible Changes

- Adds one in-repo planning package that makes the next internal runtime priorities and issue intake
  order explicit.

## Failure Recovery

- Fast rollback or disable path:
  Revert this docs-only commit.
- Observable failure symptoms reviewers should watch for:
  Priority order drift between the new docs and existing roadmap/design documents.

## Reviewer Focus

- Verify the issue-reuse logic (`#172` reused, `#45` not reused)
- Verify the priority order matches the code evidence in `alpha-test`
- Verify the docs package does not imply runtime work already landed
````
