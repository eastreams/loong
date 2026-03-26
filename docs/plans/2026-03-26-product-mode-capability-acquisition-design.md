# Product Mode Capability Acquisition Design

> Follow-up refinement: issue `#596` keeps `product mode` as the external
> external product-facing surface, but generalizes the internal runtime control
> plane
> into an autonomy-policy kernel. This document remains the product-facing
> capability-acquisition contract. See
> `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md` for the
> internal architecture refinement.

**Problem**

LoongClaw now has a materially better `discovery-first` substrate, but it still
does not expose a first-class product contract for capability acquisition.

That gap matters because `discovery-first` and `product mode` solve different
problems:

- `discovery-first` answers: "which currently visible tool should the model use
  next?"
- `product mode` answers: "when the current visible surface is insufficient, is
  the runtime allowed to acquire or switch capability, under what approval
  rules, and how should that be explained to the user?"

Issue `#570` and the focused search/discovery fix in PR `#580` showed that
better search is necessary, but not sufficient, for nanobot-style
"autonomous expansion" behavior. Search can improve recall, multilingual
matching, and follow-up stability, but it cannot by itself define:

- whether expansion is permitted
- whether expansion requires approval
- whether provider mutation is allowed
- whether the current channel/runtime binding is strong enough to support that
  behavior
- what the user sees when expansion is blocked

Without an explicit `product mode`, those decisions remain distributed across
prompt behavior, tool descriptions, and local follow-up heuristics.

## Current Architecture Evidence

The repository already contains the main building blocks needed for product
mode, but they do not yet form one explicit contract.

### Discovery-first substrate already exists

- `tool.search` and `tool.invoke` are the only provider-core discovery tools.
  Evidence:
  - `crates/app/src/tools/catalog.rs`
  - `crates/app/src/tools/mod.rs`
  - `docs/plans/2026-03-15-tool-discovery-architecture.md`
- discovery results carry lease-bearing follow-up context so provider turns can
  bridge from search into invocation.
  Evidence:
  - `crates/app/src/provider/shape.rs`
  - `crates/app/src/conversation/turn_shared.rs`

### Capability acquisition surfaces already exist

- `external_skills.fetch`
- `external_skills.install`
- `external_skills.invoke`
- `provider.switch`

These are real runtime mutation surfaces, not hypothetical future extension
points. Evidence:

- `crates/app/src/tools/external_skills.rs`
- `crates/app/src/tools/provider_switch.rs`
- `crates/app/src/tools/catalog.rs`

### Governance and approval infrastructure already exists

- tools carry governance metadata:
  - scheduling class
  - governance scope
  - risk class
  - approval mode
- governed execution can already produce approval requests and operator
  resolution flows.

Evidence:

- `crates/app/src/tools/catalog.rs`
- `crates/app/src/tools/approval.rs`
- `crates/app/src/session/repository.rs`

### Runtime binding is explicit, but still admits direct fallback

The conversation layer already has an explicit `ConversationRuntimeBinding`, but
it still allows `Direct` fallback:

- `Kernel(&KernelContext)`
- `Direct`

Evidence:

- `crates/app/src/conversation/runtime_binding.rs`
- `docs/plans/2026-03-15-conversation-runtime-binding-implementation-plan.md`
- `docs/plans/2026-03-15-provider-binding-normalization-implementation-plan.md`

That is acceptable for current discovery-first execution, but it is not a
strong enough contract for autonomous capability acquisition. A runtime that can
mutate provider state, install capability, or expand topology should not depend
on optional kernel binding.

### Channel descriptors are integration-aware, not mode-aware

Current channel descriptors identify available integrations and runtime
surfaces, but they do not yet declare which capability-acquisition behaviors
they permit.

Evidence:

- `crates/app/src/config/channels.rs`
- `crates/app/src/channel/registry.rs`
- `crates/app/src/channel/runtime_state.rs`

This becomes more important as additional channels land. Otherwise every channel
risks growing local heuristics for "how autonomous should LoongClaw be here?"

## Design Goals

1. Keep `discovery-first` as the tool-selection substrate.
2. Add a first-class `product mode` contract above discovery-first.
3. Make capability acquisition explicit, bounded, and governable.
4. Keep the design channel-agnostic and SDK-friendly.
5. Fail closed when runtime binding or approval infrastructure is not strong
   enough for the selected mode.
6. Make failure reasons operator-visible and explainable.

## Non-goals

- Do not replace the existing discovery-first tool architecture.
- Do not introduce channel-specific autonomy rules.
- Do not silently auto-install skills by prompt convention alone.
- Do not require embeddings or semantic retrieval to define the product mode.
- Do not widen this slice into full plan-IR redesign before the product
  contract exists.

## Core Idea

`product mode` should be modeled as a runtime contract that governs
capability-acquisition behavior, while `discovery-first` remains the mechanism
used to search the current visible surface.

That yields a layered model:

1. Product mode selects what categories of acquisition are allowed.
2. Discovery-first searches the currently visible tool surface.
3. If a capability gap is detected, the product-mode policy evaluates whether a
   mutation path is legal.
4. If legal, the runtime either:
   - requests approval
   - executes a bounded acquisition action
   - explains why the request is blocked
5. Discovery is refreshed after any successful acquisition or provider switch.
6. Normal task execution resumes on the updated visible surface.

## Proposed Product Modes

Start with three explicit modes.

### 1. `discovery_only`

Behavior:

- search the current visible surface
- invoke already-visible tools
- do not fetch, install, enable, or switch capability
- if a capability gap is detected, explain the block and suggest next actions

Allowed mutation classes:

- none for capability acquisition

Intended use:

- default conservative mode
- direct CLI fallback
- channels without strong approval or kernel-backed session control

### 2. `guided_acquisition`

Behavior:

- search the current visible surface
- detect capability gaps
- surface acquisition candidates
- require explicit approval before capability-acquisition actions that mutate the
  runtime or install new capability

Allowed mutation classes:

- skill fetch after approval
- skill install after approval
- skill invoke after approval or as a consequence of approved install
- provider switch after approval

Intended use:

- production channel runtimes with approval support
- operators who want assisted expansion, but not hidden autonomy

### 3. `bounded_autonomous`

Behavior:

- search the current visible surface
- detect capability gaps
- automatically perform a bounded subset of approved acquisition actions within
  strict budget and source-policy limits
- still require approval for topology mutation or high-risk mutations outside
  the configured autonomy envelope

Allowed mutation classes:

- skill fetch within allowed domains
- skill install from approved sources
- skill invoke when eligibility and invocation policy allow it
- provider switch only if explicitly enabled for autonomous mode

Still not autonomous by default:

- `delegate`
- `delegate_async`
- session mutation tools
- policy reconfiguration tools

Intended use:

- opt-in operator workflows
- bounded automation environments where the runtime is kernel-bound and approval
  semantics are available if the autonomy envelope is exceeded

## Capability Action Classes

Product mode should not reason directly over raw tool names alone. It should use
an intermediate action taxonomy derived from the tool catalog.

Proposed action classes:

- `discover`
  - `tool.search`
  - `external_skills.list`
  - `external_skills.inspect`
- `execute_existing`
  - `tool.invoke`
  - normal already-visible tools
- `capability_fetch`
  - `external_skills.fetch`
- `capability_install`
  - `external_skills.install`
- `capability_load`
  - `external_skills.invoke`
- `runtime_switch`
  - `provider.switch`
- `topology_expand`
  - `delegate`
  - `delegate_async`
- `policy_mutation`
  - `external_skills.policy`
- `session_mutation`
  - `session_cancel`
  - `session_recover`
  - `session_archive`

Canonical base mode decision table:

| action class | `discovery_only` | `guided_acquisition` | `bounded_autonomous` | notes |
| --- | --- | --- | --- | --- |
| `discover` | `allow` | `allow` | `allow` | discovery-first remains available in every mode |
| `execute_existing` | `allow` | `allow` | `allow` | product mode does not interfere with already-visible tool execution |
| `capability_fetch` | `blocked` | `approval_required` | `allow` | lower layers may still block on source policy, binding, or budget |
| `capability_install` | `blocked` | `approval_required` | `allow` | lower layers may still block on source policy, binding, or budget |
| `capability_load` | `blocked` | `approval_required` | `allow` | an approved install may reuse the same approval transaction |
| `runtime_switch` | `blocked` | `approval_required` | `approval_required` | may elevate to `allow` only when autonomous provider switching is explicitly enabled |
| `topology_expand` | `blocked` | `approval_required` | `approval_required` | never auto-allowed by product mode alone |
| `policy_mutation` | `blocked` | `approval_required` | `approval_required` | remains governed and never auto-allowed by product mode alone |
| `session_mutation` | `blocked` | `approval_required` | `approval_required` | remains governed and never auto-allowed by product mode alone |

This taxonomy lets the mode engine stay stable even as new tools or channels are
added.

This table is the canonical base product-mode decision matrix. Governance,
source-policy, channel-support, runtime-binding, and budget layers may only make
an outcome stricter. They must not weaken the base mode result.

## Policy Layering

`product mode` should not collapse every safety and execution concern into one
enum. It should sit inside a layered policy stack.

### 1. Discovery layer

- discovers the current visible surface
- decides which already-visible tool or channel surface is relevant
- remains responsible for multilingual recall, synonym handling, and coarse
  fallback on the current surface

### 2. Product-mode layer

- decides whether the runtime may cross the boundary from "visible capability"
  into "newly acquired or switched capability"
- reasons over capability action classes rather than raw tool names
- uses the canonical base decision matrix from `Capability Action Classes`
- decides between `allow`, `approval_required`, and `blocked`

### 3. Governance layer

- preserves the existing tool governance contract
- contributes risk class, approval mode, and topology-mutation scope
- prevents product mode from weakening hard governance rules

Evidence:

- `crates/app/src/tools/catalog.rs`
- `crates/app/src/tools/approval.rs`

### 4. Source-policy and eligibility layer

- enforces domain allowlists and denylists for external skill acquisition
- enforces install-root, exposure, and runtime eligibility constraints
- constrains provider switching to a resolvable runtime config path

Evidence:

- `crates/app/src/tools/runtime_config.rs`
- `crates/app/src/tools/external_skills.rs`
- `crates/app/src/tools/provider_switch.rs`

### 5. Channel and binding layer

- determines whether the current surface can support kernel-backed mutation
- determines whether approval round-trips can actually complete
- constrains which product modes are even legal on the current entry surface

Evidence:

- `crates/app/src/conversation/runtime_binding.rs`
- `crates/app/src/config/channels.rs`
- `crates/app/src/channel/registry.rs`

This layering matters because the same action can be blocked for different
reasons:

- discovery found no suitable existing capability
- product mode disallows acquisition
- governance requires approval
- source policy denies the source
- the channel surface cannot support the requested mode

Those reasons should stay distinct.

## Mode Resolution

The runtime needs one deterministic way to resolve the active product mode for a
turn.

Recommended precedence:

1. explicit operator or session override
2. channel-surface default, when the current entry surface declares one
3. global runtime default

Validation rules:

- a requested mode must be supported by the current channel surface
- a requested mode must satisfy the current runtime binding contract
- a requested mode must not silently weaken governance or source-policy
  constraints
- unsupported combinations should fail closed, not silently downgrade

This should produce one resolved outcome before capability acquisition begins:

- `resolved(mode)`
- `blocked(product_mode_unsupported_by_channel_surface)`
- `blocked(product_mode_kernel_binding_missing)`
- `blocked(product_mode_approval_roundtrip_unavailable)`

Mode resolution happens first.

If it returns `resolved(mode)`, the runtime then consults the base mode decision
matrix for that resolved mode.

Only after the base mode decision matrix yields its
`allow | approval_required | blocked` result should lower-layer restrictions or
approval execution proceed.

That is important for operator trust. Silent downgrade from
`bounded_autonomous` to `discovery_only` would make the system harder to reason
about and harder to debug.

## Product Mode State Machine

Product mode should be modeled as an explicit turn-level state machine rather
than an emergent prompt pattern.

### States

1. `surface_discovery`
   - inspect current visible tool surface
   - run `tool.search` if needed

2. `gap_evaluation`
   - determine whether the task can complete with the current surface
   - if not, classify the missing capability into an action class

3. `policy_gate`
   - consult current product mode
   - consult runtime binding strength
   - consult governance profile and approval rules
   - consult mode budget

4. `approval_pending`
   - request operator approval when the mode allows guided expansion but not
     autonomous execution

5. `capability_acquisition`
   - execute the allowed acquisition step
   - record explicit acquisition outcome

6. `surface_refresh`
   - rebuild visible surface / discovery context after acquisition or provider
     switch

7. `task_execution`
   - continue ordinary tool execution on the refreshed surface

8. `blocked_explanation`
   - emit a structured, user-visible reason when acquisition is not allowed

9. `completed`
   - task completed or terminally blocked with explanation

### Required transitions

- `surface_discovery -> gap_evaluation`
- `gap_evaluation -> task_execution` when no gap exists
- `gap_evaluation -> policy_gate` when a gap exists
- `policy_gate -> blocked_explanation` when mode disallows the action
- `policy_gate -> approval_pending` when approval is required
- `policy_gate -> capability_acquisition` when autonomous acquisition is allowed
- `approval_pending -> capability_acquisition` when approved
- `capability_acquisition -> surface_refresh` when acquisition succeeds
- `surface_refresh -> task_execution`
- any state -> `blocked_explanation` on hard policy denial or budget exhaustion

## Approval and Blocked Outcome Model

Product mode should reuse existing approval infrastructure, but it should stop
overloading every high-risk path into the same generic "governed tool requires
approval" story.

Add a mode-aware outcome reason model with two deterministic families:

Approval-required reasons:

- `product_mode_capability_fetch_requires_approval`
- `product_mode_capability_install_requires_approval`
- `product_mode_provider_switch_requires_approval`

Blocked reasons:

- `product_mode_disallows_capability_acquisition`
- `product_mode_kernel_binding_missing`
- `product_mode_approval_roundtrip_unavailable`
- `product_mode_autonomy_budget_exceeded`
- `product_mode_source_policy_denied`
- `product_mode_provider_switch_disallowed`
- `product_mode_unsupported_by_channel_surface`

Approval-required reasons should be carried by approval requests. Blocked
reasons should be carried by `blocked_explanation` outcomes.

This keeps approval requests and blocked explanations truthful. The operator
should be able to distinguish whether an action:

- requires approval before it may proceed
- is blocked because the selected mode or current surface forbids acquisition
- is blocked because the current binding is too weak
- is blocked because the current surface cannot complete the approval round-trip
- is blocked because the action exceeded the configured autonomy envelope

## Kernel Binding Contract

`bounded_autonomous` and `guided_acquisition` should not be valid on a weak
conversation binding.

Proposed rule:

- `discovery_only` may run with `ConversationRuntimeBinding::Direct`
- define `is_explanation_only_blocked_outcome(outcome)` as true only when the
  evaluator returns `blocked(reason_code)` and emits no approval request or
  mutation step
- `guided_acquisition` requires a kernel-bound conversation unless
  `is_explanation_only_blocked_outcome(outcome)` returns true
- `bounded_autonomous` requires kernel binding and must fail closed otherwise

This is the main reason product mode belongs above discovery-first. Discovery
itself can degrade to direct mode; autonomous capability acquisition should not.

## Channel / SDK Contract

Channel integration descriptors should declare product-mode support rather than
leaving autonomy behavior implicit.

Today those descriptors are split across configuration-facing channel metadata
and runtime-facing channel registry metadata. Product-mode support should attach
to those existing surfaces, or to a future unified SDK descriptor that replaces
them, rather than appearing as channel-local prompt behavior.

Proposed descriptor additions:

- default product mode
- allowed product modes
- whether operator approval round-trips are supported
- whether kernel-bound execution is guaranteed
- whether autonomous capability acquisition is permitted on that surface
- whether session-level product-mode overrides are permitted on that surface

This keeps future channel integrations from re-implementing local autonomy
semantics.

## User-visible Semantics

Every non-`discovery_only` mode should produce explicit operator-visible
behavior:

- current product mode
- why an acquisition path was selected
- whether approval is required
- whether an action was auto-executed because the mode allowed it
- why the runtime refused to proceed when blocked

Use the canonical blocked reason list from `Approval and Blocked Outcome Model`
as the only product-mode blocked reason source of truth. UI surfaces should map
those codes directly rather than redefining a second list here.

## Budget Model

Autonomous expansion must be bounded.

Minimum budgets:

- max acquisition actions per turn
- max acquisition rounds per turn
- max provider switches per turn
- max distinct capability installs per turn
- optional cooldown against repeated failed acquisition of the same target

The budget should be mode-scoped and deterministic.

Budget enforcement should happen after mode resolution and before mutation
execution. It should not be embedded in prompt instructions.

## Why This Is Better Than Extending Discovery-first Alone

Improving search recall does not solve:

- approval visibility
- autonomous mutation policy
- channel/runtime opt-in
- budget enforcement
- operator-visible failure semantics
- binding strength requirements

Discovery-first should remain the selection mechanism. Product mode should own
the acquisition policy.

## Integration Strategy

This design intentionally does not require immediate plan-IR expansion.

Phase 1 can be implemented around the current turn loop by:

- classifying capability action classes in the tool catalog
- resolving product mode from session override, channel support, and runtime
  default
- adding a small mode-policy evaluator
- attaching mode state to session/runtime context
- routing approval or blocked explanations before mutation actions execute

Later phases may extend `PlanNodeKind` with:

- `AcquireCapability`
- `AwaitApproval`
- `RefreshDiscovery`

but that should happen after the product contract exists.

## Risks

### Risk: treating product mode as prompt text

Rejected because prompt-only mode selection will drift across providers and
channels.

### Risk: channel-specific autonomy rules

Rejected because the problem is architectural, not channel-local.

### Risk: silently auto-installing skills

Rejected because it weakens operator trust and makes failure semantics
untruthful.

### Risk: overloading existing governed-tool approval reasons

Rejected because it obscures the real reason an action is blocked.

## Recommended Direction

Adopt `product mode` as an explicit runtime contract with three starting modes:

- `discovery_only`
- `guided_acquisition`
- `bounded_autonomous`

Treat discovery-first as a lower-layer execution substrate, not the top-level
product behavior.

That is the smallest structurally correct way to support future autonomous
expansion without falling back into hidden prompt behavior or channel-specific
patching.
