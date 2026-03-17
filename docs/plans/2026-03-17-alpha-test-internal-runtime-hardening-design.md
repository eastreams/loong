# Alpha-Test Internal Runtime Hardening Design

Date: 2026-03-17
Branch: `docs/internal-runtime-hardening`
Scope: choose the next internal hardening program for `alpha-test` and prepare it for clean GitHub issue/PR intake
Status: approved internal roadmap and backlog direction

## Problem

`alpha-test` already looks more like a real kernel than most agent repositories:

1. the architecture contract is explicit and mechanically enforced
2. kernel capability, policy, and audit seams are real
3. the repository already carries design docs, issue-first discipline, and CI parity gates

That is the good news.

The remaining risk is subtler and more important than adding one more feature surface. The branch
still has a truthfulness gap between its strongest architectural claims and a few production-shaped
runtime seams:

1. [Core Beliefs](../design-docs/core-beliefs.md) says all execution paths are kernel-first with no
   shadow paths, but the app runtime still normalizes explicit `Direct` compatibility lanes through
   `ConversationRuntimeBinding` and `ProviderRuntimeBinding`.
2. [Security](../SECURITY.md) explicitly says connector, ACP, and runtime-only analytics are still
   not uniformly routed through the L1 policy chain.
3. the kernel already owns a clean `AuditSink` seam, but
   [`crates/kernel/src/audit.rs`](../../crates/kernel/src/audit.rs) is still in-memory only.
4. the ACP control plane is real and valuable, but the complexity budget is now concentrated in a
   few very large hotspots:
   - `crates/app/src/acp/manager.rs` at 3063 lines
   - `crates/app/src/acp/acpx.rs` at 2574 lines
   - `crates/app/src/conversation/turn_coordinator.rs` at 8613 lines

This is the wrong point to optimize for breadth. The next program should harden the runtime's
truthfulness, evidence durability, and control-plane decomposition before LoongClaw spends more of
its complexity budget on outward-facing product surfaces.

## Goals

1. Pick the next 4-5 internal workstreams that most improve runtime truthfulness and operator
   confidence on `alpha-test`.
2. Reuse the repository's existing design momentum instead of opening duplicate or contradictory
   tracks.
3. Turn that direction into GitHub-ready artifacts:
   - a docs issue draft for this planning package
   - one umbrella feature issue
   - child issue drafts for new workstreams
   - one PR draft for the current documentation package
4. Keep the plan honest about what is already strong versus what is still soft.

## Non-Goals

1. Do not optimize README or external onboarding in this slice.
2. Do not copy a competitor feature matrix into LoongClaw.
3. Do not reopen already-closed broad policy work just because related gaps remain.
4. Do not treat productization or workflow packs as the first next move.
5. Do not redesign the entire repository in one mega-refactor.

## Current State And Evidence

### 1. Kernel direction is already strong

The repository's kernel shape is not hypothetical:

1. `crates/kernel/src/kernel.rs` is a real orchestration boundary for runtime, tool, memory, and
   connector execution.
2. the repository enforces the crate DAG and architecture boundaries mechanically
3. `docs/design-docs/layered-kernel-design.md` already defines L1-L9 responsibilities with a clear
   kernel-first bias

This matters because the right next step is to finish the strongest story LoongClaw already has,
not to pivot away from it.

### 2. Explicit compatibility lanes still exist

The branch has already improved from implicit `Option<&KernelContext>` ambiguity to explicit
runtime bindings, but it has not fully retired governed-versus-direct drift:

1. `crates/app/src/conversation/runtime_binding.rs` still models `ConversationRuntimeBinding` as
   `Kernel` or `Direct`
2. `crates/app/src/provider/runtime_binding.rs` does the same for provider execution
3. `docs/SECURITY.md` still documents intentional non-uniform L1 coverage for connector, ACP, and
   runtime-only analytics

That is a valid compatibility position, but it is not yet the end-state architecture implied by
the core beliefs.

### 3. Durable audit is the clearest kernel evidence gap

The audit seam is already well-shaped:

1. `AuditSink` is small and kernel-owned
2. kernel operations already propagate sink errors
3. the repository already has a concrete design for durable audit in
   `docs/plans/2026-03-15-persistent-audit-sink-design.md`

The gap is not design ambiguity. The gap is that the durable path still has not landed, and issue
`#172` is already open for it.

### 4. ACP is strategically important but now needs hardening more than expansion

The ACP pre-embed work is directionally correct. LoongClaw already distinguishes:

1. ACP control plane
2. ACP runtime backends such as ACPX
3. explicit dispatch policy
4. session-aware routing and observability

That is the right shape. The risk is not "ACP should go away." The risk is that further ACP growth
on top of the current file-size and recovery complexity will accumulate avoidable control-plane
debt.

### 5. Provider runtime decomposition is the right internal model to copy

`crates/app/src/provider/mod.rs` is down to 294 lines because the provider runtime has already been
decomposed into focused runtime and policy modules. That is a proof point inside this same branch:
LoongClaw already knows how to reduce a large orchestration hotspot without losing behavior.

ACP should follow that style instead of staying concentrated in one manager file and one backend
file indefinitely.

## External Calibration

The comparison target is not a shopping list. It is a calibration exercise against the primary
repository surfaces of these projects as seen on 2026-03-17:

1. OpenClaw emphasizes a strong gateway/control-plane model, multi-channel session orchestration,
   and ACP-aware runtime behavior.
2. nanobot emphasizes radical simplicity and low implementation weight.
3. NanoClaw emphasizes container isolation and understandable secure execution.
4. ZeroClaw emphasizes a small Rust runtime with secure-by-default positioning.
5. OpenFang emphasizes operator OS packaging, durable audit language, and prebuilt autonomous
   capability packages.

The useful inference is not "LoongClaw should become all of them." The useful inference is:

1. LoongClaw already has stronger architecture governance than the lightweight clones.
2. LoongClaw is directionally aligned with the strongest systems on kernel/runtime boundaries.
3. LoongClaw still lags the strongest operator platforms on durable evidence, execution packaging,
   and control-plane polish.

That makes the next move clear: finish the kernel truth and runtime hardening first, then
productize from a stronger base.

## Approaches Considered

### A. Product-surface-first push

Prioritize workflow packs, browser-first UX, and operator-facing packaging immediately.

Rejected.

This would spend complexity on the most visible surfaces before the underlying execution truth is
fully hardened. It risks building a glossy layer over runtime seams that are still intentionally
soft.

### B. Single-gap hardening only

Do only one slice such as persistent audit or only one more runtime-binding fix.

Rejected as the main program direction.

Each slice is good on its own, but the repository now needs a coherent near-term program, not a
pile of disconnected "good ideas."

### C. Internal runtime hardening program

Prioritize:

1. kernel-first runtime closure
2. durable audit retention and operator queryability
3. ACP control-plane decomposition and recovery
4. cross-lane execution tiers
5. first-party workflow packs only after the runtime base is harder

Recommended.

This keeps LoongClaw on its strongest line of development and uses existing repo momentum instead
of inventing a new identity mid-stream.

## Decision

Adopt Approach C.

The next LoongClaw program should be an internal runtime hardening track with one umbrella issue
and four primary child streams:

1. Kernel-first runtime closure and direct-path retirement
2. Persistent kernel audit sink and query baseline
3. ACP control-plane lifecycle hardening and observability
4. Cross-lane execution security tiers

First-party workflow packs remain important, but only as the fifth stream after the runtime base is
harder and more truthful.

## Program Streams

### Stream 1: Kernel-First Runtime Closure

Priority: P0

This stream exists to finish the architectural truth that the repo already claims.

Target outcome:

1. `Direct` remains only as an explicit compatibility seam at ingress, tests, or intentionally
   unsupported paths
2. deep orchestration code no longer silently slides from governed to direct behavior
3. governed reads and governed side effects fail closed instead of silently re-entering shadow paths

Likely file surfaces:

1. `crates/app/src/conversation/runtime_binding.rs`
2. `crates/app/src/provider/runtime_binding.rs`
3. `crates/app/src/conversation/turn_coordinator.rs`
4. `crates/app/src/conversation/session_history.rs`
5. `crates/app/src/chat.rs`
6. `docs/SECURITY.md`
7. `ARCHITECTURE.md`

Existing repo momentum to reuse:

1. `docs/plans/2026-03-15-conversation-runtime-binding-design.md`
2. `docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md`
3. `docs/plans/2026-03-16-governed-runtime-path-hardening-design.md`
4. `docs/plans/2026-03-15-provider-binding-normalization-design.md`

Why it comes first:

Because runtime truthfulness is the highest-value improvement LoongClaw can make before adding more
operator surface area.

### Stream 2: Persistent Audit Sink And Query Baseline

Priority: P1

This stream should reuse existing open issue `#172` rather than opening a duplicate.

Target outcome:

1. security-critical audit evidence survives restart
2. production-shaped bootstraps stop defaulting to process-local-only evidence
3. operators get a minimal local inspection/query surface

Likely file surfaces:

1. `crates/kernel/src/audit.rs`
2. `crates/kernel/src/lib.rs`
3. `crates/kernel/src/tests.rs`
4. `crates/app/src/config/runtime.rs`
5. `crates/app/src/context.rs`
6. `crates/spec/src/kernel_bootstrap.rs`
7. `docs/SECURITY.md`
8. `docs/RELIABILITY.md`

Why it is second:

It is the cleanest kernel-owned evidence gap, the design is already written, and the issue is
already open. It should move immediately after the kernel-closure umbrella is in place.

### Stream 3: ACP Control-Plane Hardening And Recovery

Priority: P2

This stream is not about adding more ACP concepts. It is about making the existing ACP stack easier
to reason about, recover, and observe.

Target outcome:

1. `AcpSessionManager` is decomposed into smaller ownership boundaries
2. ACPX transport/process management is separated from control-plane semantics
3. stuck-turn recovery, cancel/close repair, and status observability become explicit and testable
4. ACP runtime events have a durable story instead of only a process-local one

Likely file surfaces:

1. `crates/app/src/acp/manager.rs`
2. `crates/app/src/acp/acpx.rs`
3. `crates/app/src/acp/backend.rs`
4. `crates/app/src/acp/store.rs`
5. `crates/app/src/conversation/turn_coordinator.rs`
6. `docs/design-docs/acp-acpx-preembed.md`

Why it is third:

ACP is already strategically valuable. The next best move is hardening and decomposition, not more
surface-area growth.

### Stream 4: Cross-Lane Execution Security Tiers

Priority: P3

LoongClaw already names sandbox tiers in the roadmap, but the execution story still needs one
shared tier model across process, browser, and WASM-style lanes.

Target outcome:

1. one vocabulary for `restricted`, `balanced`, and `trusted` execution
2. policy defaults and runtime evidence are aligned across bridges
3. browser/process/WASM lanes stop drifting into lane-specific security semantics

Likely file surfaces:

1. `crates/kernel/src/policy.rs`
2. `crates/kernel/src/runtime.rs`
3. bridge execution surfaces in `crates/spec`
4. browser/runtime integration surfaces in `crates/daemon`
5. `docs/SECURITY.md`
6. `docs/ROADMAP.md`

Why it is fourth:

This is the right time to unify lane semantics, but only after kernel truth and durable evidence are
stronger.

### Stream 5: First-Party Workflow Packs

Priority: P4

This remains the right productization direction after the runtime base is harder.

Target outcome:

1. 2-3 first-party workflow packs that show the value of the kernel
2. packs build on hardened execution, audit, and control-plane boundaries instead of compensating
   for weak ones

Possible starting packs:

1. release/review operator pack
2. issue triage and maintenance pack
3. channel support or customer-ops pack

Why it is last in this program:

Because this is where LoongClaw should cash in the runtime investment, not where it should borrow
against it.

## Sequencing

Recommended execution order:

1. open one docs issue and ship the current planning PR
2. open one umbrella runtime-hardening feature issue
3. open a new Stream 1 issue because earlier broad policy-unification issue `#45` is closed
4. reuse open issue `#172` for Stream 2
5. open new issues for Stream 3 and Stream 4
6. leave Stream 5 as a later issue after Streams 1-4 are underway

## Why This Direction Is Better Than More Surface Breadth

This program intentionally resists "slop debt."

It does not propose:

1. a large unscoped rewrite
2. cargo-culted competitor features
3. product breadth ahead of runtime truth
4. duplicate issues for work that the repo is already tracking clearly

Instead it builds on the repository's strongest qualities:

1. kernel-first architecture
2. strong design-doc culture
3. explicit control over complexity growth
4. a preference for small, composable, reviewable slices

## Acceptance Criteria

This design is successful if it produces and aligns all of the following:

1. a written internal runtime hardening direction grounded in current repo evidence
2. a prioritized execution order that explains why each stream is where it is
3. explicit reuse of open issue `#172` and explicit non-reuse of closed issue `#45`
4. GitHub-ready issue and PR drafts for the planning package and the next workstreams
5. a roadmap update that reflects the same priority order
