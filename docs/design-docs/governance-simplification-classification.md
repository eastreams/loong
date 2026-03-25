# Governance Simplification Classification

## Scope and Baseline

- Baseline repository: `loongclaw-ai/loongclaw`
- Baseline branch and commit: `dev@5af679b6` on 2026-03-25
- Driving issue: `#458`
- Source discussion: `#402`
- Related implementation tracks: `#457`, `#196`, `#48`

This document answers a narrow question:

Which governance surfaces are still structural today, which ones are only
transitional, which ones are safe to simplify now, and which ones must be
replaced before they can be deleted?

It does not propose a repo-wide rewrite. It also does not reopen the broad
"delete the whole current governance stack" framing from `#402`. The goal is
to make future simplification work truthful, bounded, and sequence-aware.

## Why This Exists

`#402` correctly identified that parts of the current governance story are
heavier than the runtime behavior they presently protect. That concern is real.
It is not sufficient, however, to justify a blanket delete-first cleanup.

Current repo evidence shows a mixed picture:

- Some governance surfaces are low-evidence or cleanup-shaped and can be
  handled in narrow slices.
- Some surfaces are still the live contract that keeps the kernel-first story
  true enough to continue hardening it.
- Some surfaces are explicit compatibility lanes that should shrink, but should
  not disappear before a replacement exists.
- Some surfaces are not simplification targets at all. They are control-plane
  or long-term replacement work and need their own track.

That is why `#457` and `#458` were split out of `#402`.

## Classification Vocabulary

| Class | Meaning | Default action |
| --- | --- | --- |
| `live structural` | The surface still carries real contract or enforcement weight in the current runtime. | Keep it. Simplify only around it, not through it. |
| `transitional` | The surface is an explicit compatibility or migration seam. | Keep it narrow, documented, and non-expanding. |
| `safe to simplify now` | The surface can be reduced or removed in bounded cleanup slices without waiting for a new model. | Handle through focused proof-backed cleanup work. |
| `replace before delete` | The surface is imperfect, but it still carries semantics that the repo would lose if it vanished now. | Build the replacement first, then delete. |

## Current Repo Facts That Matter

These facts drive the classification below.

### 1. The workspace shape is still strong

The current workspace remains a strict 7-crate DAG with no dependency cycles:

```text
contracts
├── kernel
├── app
├── spec
├── bench
└── daemon

protocol
└── spec
```

This matters because the governance story still has a clean structural home:
`contracts` owns the contract surface, `kernel` owns the enforcement path, and
`app` carries the integration and compatibility seams.

### 2. The kernel still owns the main governance contract

`crates/contracts/src/lib.rs` still exports the main governance vocabulary:

- `Capability`
- `CapabilityToken`
- `PolicyContext`
- `PolicyDecision`
- `PolicyRequest`
- `VerticalPackManifest`

`crates/kernel/src/policy.rs` and `crates/kernel/src/kernel.rs` still wire that
vocabulary into real runtime behavior:

- token issuance
- token expiry and pack checks
- policy authorization
- policy extension evaluation
- audit recording
- execution-plane entrypoints

This is not dead scaffolding in the current branch. It is the live shape of the
kernel-side authority model.

### 3. The repo already admits that direct-mode runtime drift still exists

The repo is explicit that the long-term architecture wants less deep direct
mode:

- `ARCHITECTURE.md` describes the project as kernel-first.
- `docs/design-docs/core-beliefs.md` says all execution paths should route
  through the kernel.
- `docs/ROADMAP.md` already tracks `D6: Retire governed/direct runtime drift`.

That means the remaining direct-mode surfaces should be treated as migration
seams, not as proof that the kernel governance model is disposable.

### 4. Production audit is no longer purely in-memory

`crates/app/src/context.rs` now defaults production-facing bootstraps to the
configured audit mode and can build:

- `JsonlAuditSink`
- `FanoutAuditSink`
- `InMemoryAuditSink`

This means the audit abstraction is still a real part of the current runtime
contract, even though some narrow in-memory and noop seams remain for fixtures
or explicit opt-in paths.

### 5. Compatibility seams are explicit and still visible

The app layer now makes governed versus direct mode explicit through:

- `ConversationRuntimeBinding`
- `ProviderRuntimeBinding`
- a small number of outer `Option<&KernelContext>` wrappers that normalize into
  bindings

The audit pass for `#458` still found `optional_kernel_context` hotspots and
binding-first seams in:

- `crates/app/src/channel/mod.rs`
- `crates/app/src/conversation/runtime_binding.rs`
- `crates/app/src/conversation/session_history.rs`
- `crates/app/src/conversation/turn_coordinator.rs`
- `crates/app/src/conversation/turn_engine.rs`
- `crates/app/src/provider/runtime_binding.rs`

Those are explicit migration surfaces, not evidence that the kernel-first path
has already been removed.

### 6. ACP is a real control plane, not simplification noise

The control-plane audit found concentrated ACP complexity, including high counts
of:

- locks
- atomics
- timers
- process-command sites
- `tokio::spawn` sites

That is strong evidence that ACP is now a real subsystem. It may need
decomposition and hardening, but it is not a candidate for casual governance
deletion.

### 7. Feature flags still change governance-adjacent semantics

The feature-flag audit still shows heavy `memory-sqlite` concentration inside
the app runtime. That matters because some history, diagnostic, and
continuity-related behavior changes with feature composition.

This is another reason to avoid broad delete-first simplification. A governance
surface that only looks redundant in one feature slice may still carry behavior
in another.

## Surface Classification

### Structural Surfaces That Must Stay

| Surface | Why it is still live | Class | Recommendation |
| --- | --- | --- | --- |
| `Capability`, `CapabilityToken`, `Policy*` request/decision types, and `VerticalPackManifest.granted_capabilities` | They still define the current kernel authority contract exported from `contracts` and consumed by execution entrypoints. | `live structural` | Keep stable until a replacement model is both implemented and adopted. |
| `StaticPolicyEngine`, `PolicyEngine::authorize`, and `PolicyExtensionChain` | They still gate token validity, pack matching, capability checks, and extension-time tightening. The deprecated `check_tool_call` hook is not the live path anymore, but the authorization stack is. | `live structural` | Tighten call-site coverage through `#196` follow-up work rather than deleting the stack. |
| Kernel plane entrypoints such as `execute_tool_core`, `execute_memory_core`, `execute_runtime_core`, and connector dispatch | These functions are where the current kernel-first claim becomes real: policy and audit attach here. | `live structural` | Preserve them as the current enforcement spine. |
| `AuditSink`, `JsonlAuditSink`, and `FanoutAuditSink` | Production-facing runtime bootstrap already depends on them for durable or fanout retention. | `live structural` | Keep. Continue hardening the durable lane instead of collapsing the abstraction. |
| `InMemoryAuditSink` | It is no longer the production default, but it remains structural for test, harness, and side-effect-free snapshot scenarios. | `live structural` | Keep as a narrow non-production seam. Do not let it regain production default status. |

### Transitional Surfaces That Should Shrink, Not Vanish Blindly

| Surface | Current role | Class | Recommendation |
| --- | --- | --- | --- |
| `KernelContext` in `crates/app/src/context.rs` | App-owned bridge from runtime entrypoints into the current kernel authority model. | `transitional` | Keep for now, but avoid expanding it into a second parallel governance system. |
| `ConversationRuntimeBinding` and `ProviderRuntimeBinding` | They make governed versus direct mode explicit instead of hiding `None` semantics inside deeper runtime code. | `transitional` | Keep the binding-first shape. Push direct behavior outward over time. |
| Outer `Option<&KernelContext>` compatibility wrappers | A small number of ingress-facing helpers still normalize optional kernel authority into an explicit binding. | `transitional` | Keep only where the wrapper immediately normalizes into a binding. Avoid carrying raw optional authority deeper. |
| `KernelBuilder<P> = LoongClawKernel<P>` and `.build() -> Kernel<P>` | Additive migration seam inside the kernel surface. | `transitional` | Keep it small and forward-compatible. Do not treat it as proof that handle-model replacement has already happened. |
| `NoopAuditSink` | Explicit drop-audit lane reserved for narrow fixture paths. | `transitional` | Keep only as an explicit opt-in seam. It should not spread into production-shaped runtime code. |

### Surfaces That Are Safe To Simplify Now

| Surface | Why it is safe | Class | Recommendation |
| --- | --- | --- | --- |
| Low-evidence leaf surfaces isolated by `#457` | These are cleanup candidates whose value depends on actual call-site proof, not on the overall architecture story. | `safe to simplify now` | Continue the `#457` style of narrow, proof-backed cleanup instead of bundling them into broad architectural deletion. |
| Documentation drift between kernel-first claims and remaining compatibility seams | This is a truthfulness problem, not a replacement problem. | `safe to simplify now` | Keep architecture docs honest whenever runtime semantics change. |
| Deep direct-mode routing that no longer needs to remain below ingress | The repo already tracks this as `docs/ROADMAP.md` `D6`. | `safe to simplify now` | Tackle it in bounded closure slices that move `Direct` toward explicit ingress wrappers and fail closed on governed operations. |

The key constraint is scope: "safe to simplify now" does not mean "delete the
whole current governance model". It means the work can be done without waiting
for `#48`.

### Surfaces That Require Replacement Before Deletion

| Surface | Why deletion would be wrong today | Class | Recommendation |
| --- | --- | --- | --- |
| The Token/ACL governance model as a whole | The repo still uses it as the live kernel-side authority contract. Removing it now would leave a hole, not a simplification. | `replace before delete` | Treat `#48` as the replacement track. Only delete after additive handle-model adoption reaches the app layer. |
| `KernelContext` as the app-facing authority carrier | It is not the desired long-term end-state, but app code still relies on it as the current authority object. | `replace before delete` | Replace with the future handle model before attempting removal. |
| ACP governance-adjacent control-plane surfaces | ACP is a live subsystem with real lifecycle and process semantics. It is not redundant governance scaffolding. | `replace before delete` | Decompose and harden it under its own track. Do not mix it into governance cleanup. |
| Feature-flag-conditioned governance semantics, especially memory-bound runtime behavior | Some semantics still differ by feature composition. Removing the wrong seam early would create hidden contract drift. | `replace before delete` | First converge on the stable behavior you want. Then remove the flag-shaped semantic split. |

## Recommended Sequencing

The right sequencing is not "delete everything that looks heavy". It is:

### 1. Keep low-evidence cleanup narrow

Use `#457`-style slices for:

- independently proven dead enum variants
- dead helper branches
- redundant compatibility wrappers with no remaining semantic weight

These are cleanup tasks, not architecture-redefinition tasks.

### 2. Use `#196` for the next real simplification track

The next broad simplification should not target the whole Token/ACL stack.
It should target the known runtime drift:

- push `Direct` back toward ingress
- close remaining kernel-bypass lanes
- keep governed reads and governed side effects fail-closed
- keep doc claims aligned with actual behavior

That is the simplification path that improves truthfulness without outrunning
the live runtime.

### 3. Keep audit hardening and ACP hardening on their own tracks

Durable audit and ACP decomposition are important, but they are not evidence
that the governance model is dead. They are separate hardening tracks and
should stay separate.

### 4. Treat `#48` as replacement-first architecture work

The Object-Capability Handle model from `#48` is the plausible long-term
replacement for large parts of the current authority stack.

That replacement track should remain explicit:

- additive replacement first
- app-layer migration second
- deletion of legacy ACL/token surfaces only after the new carrier is real

Anything else would create architecture churn without a stable landing zone.

## Decision Summary

The current governance story is mixed, not uniformly redundant.

What is true today:

- The kernel-side capability, token, policy, and audit contract is still
  structural.
- The app layer still carries explicit compatibility seams that should shrink.
- Narrow cleanup work is still worthwhile, but only with local proof.
- ACP and feature-flag semantic drift are separate hardening concerns.
- `#48` is the right place to discuss deleting the current authority model,
  because only that track proposes a genuine replacement.

Therefore the next-step simplification policy is:

1. simplify low-evidence leaf surfaces now
2. simplify deep direct-mode drift next
3. keep structural governance surfaces in place
4. replace before deleting the current authority carrier

That is the smallest truthful path forward after the low-evidence cleanup split.
