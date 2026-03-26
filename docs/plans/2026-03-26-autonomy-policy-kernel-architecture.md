# Autonomy Policy Kernel Architecture

> Related:
> - issue `#596`
> - product-facing baseline: `docs/plans/2026-03-26-product-mode-capability-acquisition-design.md`
> - memory-pipeline self-evolution RFC: issue `#455`

**Problem**

`#581` and merged PR `#582` established an important boundary:

- `discovery-first` selects tools on the current visible surface
- `product mode` explains how capability acquisition should look to operators

That is a strong product-facing contract, but it is still too coarse to serve
as LoongClaw's long-term internal autonomy control plane.

If the runtime keeps adding more behavior directly onto `product mode`, one
product-facing enum will eventually become a catch-all for multiple orthogonal
concerns:

- capability-acquisition permissions
- approval requirements
- kernel-binding requirements
- source-policy constraints
- channel-surface constraints
- budget enforcement
- topology-expansion boundaries
- future learning-time ranking
- future governed evolution and promotion

This becomes a structural problem as soon as LoongClaw wants to support:

- more channels and SDK surfaces
- more external-skill and provider mutation paths
- learning systems that rank or select among allowed actions
- governed self-improvement that proposes and validates new strategies

The missing layer is an internal autonomy-policy kernel underneath
`product mode`, so product modes stay simple and operator-facing while the
runtime gets a more expressive, testable, and evolvable control plane.

## Current Architecture Evidence

The existing repository already has the main seams needed to support an
autonomy-policy kernel.

### Discovery-first already exists as a lower-layer substrate

Evidence:

- `crates/app/src/tools/catalog.rs`
- `crates/app/src/tools/mod.rs`
- `crates/app/src/tools/tool_search.rs`
- `docs/plans/2026-03-15-tool-discovery-architecture.md`

The runtime can already search the visible tool surface and bridge search
results into follow-up invocation.

### Governed mutation surfaces already exist

Evidence:

- `crates/app/src/tools/external_skills.rs`
- `crates/app/src/tools/provider_switch.rs`
- `crates/app/src/tools/approval.rs`
- `crates/app/src/session/repository.rs`

LoongClaw already has real mutation paths for:

- fetching external skills
- installing external skills
- loading external skills into runtime context
- switching provider state
- requesting and resolving governed approvals

### Runtime binding is explicit

Evidence:

- `crates/app/src/conversation/runtime_binding.rs`

The runtime already distinguishes kernel-bound execution from direct execution.
That makes it possible to encode hard autonomy requirements without falling back
to prompt conventions.

### Channel SDK surfaces already exist

Evidence:

- `crates/app/src/channel/sdk.rs`
- `crates/app/src/channel/registry.rs`
- `crates/app/src/config/channels.rs`

This matters because future autonomy policy cannot live only in one product
profile enum. It must respect whether the current channel surface can support:

- background runtime control
- approval round-trips
- kernel-backed execution
- session-level policy overrides

## Research Calibration

The goal is not to copy external systems directly. The goal is to identify
which design shapes are compatible with LoongClaw's current governed runtime.

### Hermes Agent

Primary sources:

- https://github.com/NousResearch/hermes-agent
- https://hermes-agent.nousresearch.com/docs/developer-guide/architecture/
- https://hermes-agent.nousresearch.com/docs/user-guide/features/skills/
- https://hermes-agent.nousresearch.com/docs/user-guide/features/memory/

Useful ideas to learn from:

- one core agent can serve multiple user surfaces without redefining the core
  loop per surface
- skills act as procedural memory rather than only static tool metadata
- memory is persistent and useful, but still bounded and inspectable
- RL and environment tooling live around the agent stack rather than replacing
  hard execution boundaries
- security is treated as layered runtime policy, not just prompt discipline

What LoongClaw should borrow:

- keep one autonomy-policy kernel across channels
- treat learned procedures and reusable strategies as artifacts, not prompt fog
- make memory and skill growth useful, but keep it bounded and inspectable
- keep the learning stack adjacent to the agent core, not fused into hard
  permission decisions

What LoongClaw should not copy directly:

- implicit policy mutation through accumulated skills or memory alone
- making hard runtime permissions depend on soft procedural learning
- treating cross-surface consistency as a UI concern instead of a kernel concern

### HyperAgents

Primary sources:

- https://arxiv.org/abs/2603.19461
- https://github.com/facebookresearch/Hyperagents

Useful ideas to learn from:

- the improvement process itself can be represented explicitly, not hidden in
  ad hoc prompting
- persistent memory and performance tracking are valuable for cross-task
  improvement
- a system improves faster when it can reason over both task execution and
  meta-level improvement artifacts

What LoongClaw should borrow:

- model "how the agent improves" as a first-class plane, not an accidental
  byproduct of tool use
- persist evaluation artifacts and performance signals that can support later
  policy promotion
- keep meta-level proposals explicit enough to replay, compare, and audit

What LoongClaw should not copy directly into the live runtime path:

- unrestricted self-modification of the active control plane
- ungoverned online mutation inside the same turn loop that serves users
- blending experimental improvement logic with production permission decisions

The key lesson is structural:

- Hermes suggests a strong artifact and runtime boundary discipline
- HyperAgents suggests that the improvement process itself deserves explicit
  modeling

Combined, those imply that LoongClaw should separate:

- product-facing presets
- hard autonomy policy
- learning-time ranking
- governed evolution and promotion

## Design Goals

1. Keep `product mode` as the external product/profile surface.
2. Introduce an internal autonomy-policy kernel as the real runtime control
   plane.
3. Preserve deterministic hard constraints for approval, binding, source
   policy, and channel support.
4. Create a clean place for future learning systems to rank allowed actions.
5. Create a separate governed evolution plane for proposing and promoting policy
   changes.
6. Keep the design compatible with the current discovery-first runtime.

## Non-goals

- Do not remove `product mode` from the operator-facing surface.
- Do not implement the full learning or RL stack in the first slice.
- Do not allow unrestricted online self-modification of live runtime policy.
- Do not merge memory-pipeline evolution into this design; that remains related
  but separate work under issue `#455`.
- Do not turn channel SDK surfaces into autonomy-specific heuristic engines.

## Core Thesis

LoongClaw should use three different layers instead of one overloaded
`product mode` abstraction.

### 1. Product profile

This is the operator-facing layer.

Examples:

- `discovery_only`
- `guided_acquisition`
- `bounded_autonomous`

Responsibilities:

- provide understandable presets
- give channels and operators a small number of clear modes
- communicate high-level behavior

Non-responsibilities:

- encoding every hard constraint directly
- serving as the only internal runtime policy abstraction

### 2. Autonomy-policy kernel

This is the internal runtime control plane.

Suggested core types:

- `AutonomyProfile`
- `AutonomyPolicySnapshot`
- `PolicyDecisionInput`
- `PolicyDecisionOutcome`
- `CapabilityActionClass`

Responsibilities:

- compile the selected product profile into deterministic policy fields
- enforce hard constraints before mutation paths execute
- produce explicit allow, approval-required, or deny outcomes

### 3. Learning and evolution planes

These are higher layers that sit on top of the autonomy-policy kernel.

Responsibilities:

- learning plane:
  - rank or choose among already-allowed actions
  - learn from performance signals without mutating hard policy directly
- evolution plane:
  - propose changes to profiles, policies, or strategies
  - evaluate them in replay, shadow, or experiments
  - promote only through governed evidence-backed steps

## Proposed Stack

### Layer A: Discovery substrate

Input:

- current visible tools
- current conversation context

Output:

- discovery candidates on the current surface

This remains the home of multilingual search recall, synonym handling, and
coarse fallback over the visible surface.

### Layer B: Product profile

Input:

- session override
- channel default
- global runtime default

Output:

- selected product profile

This is the human-facing abstraction, not the final execution contract.

### Layer C: Autonomy-policy snapshot

Input:

- selected product profile
- runtime config
- channel support
- binding requirements

Output:

- deterministic policy snapshot for the current turn

Suggested policy fields:

- allowed capability action classes
- approval policy
- kernel-binding policy
- source policy
- provider-switch policy
- topology-expansion policy
- budget limits
- explanation policy

### Layer D: Decision engine

Input:

- policy snapshot
- capability action class
- governance profile
- binding strength
- channel support facts
- turn budget state

Output:

- `allow`
- `approval_required`
- `deny(reason_code)`

This layer should be deterministic and directly testable.

### Layer E: Capability graph

The runtime should reason over capability families rather than raw tool names
alone.

Suggested node families:

- visible tool
- external skill package
- loaded skill
- provider profile
- channel surface
- delegate child runtime

Suggested edge families:

- discover
- invoke
- fetch
- install
- load
- switch
- delegate
- mutate_policy
- future experiment promotion

This graph does not need to be a heavyweight database. It can start as typed
runtime relationships derived from the tool catalog, runtime config, and channel
surfaces.

### Layer F: Learning plane

This layer may optimize among permitted choices.

Good candidates for learning:

- search query reformulation
- ranking of discovery candidates
- ranking of provider-switch candidates
- ranking of acquisition order
- budget allocation hints
- choosing whether to reuse an installed skill or search again

This layer must not decide:

- approval bypass
- source allowlist changes
- kernel-binding relaxation
- topology-expansion rights
- live policy mutation

### Layer G: Governed evolution plane

This layer proposes and validates changes to autonomy behavior over time.

Suggested workflow:

1. propose candidate profile, policy, or strategy change
2. run static validation and invariants
3. execute replay, shadow, or bounded experiment
4. compare evidence against baseline
5. promote or reject
6. preserve rollback path

This is where HyperAgents-style meta-improvement ideas belong in LoongClaw.
They do not belong in the live turn-time permission path.

## Hard Constraints vs Learnable Behavior

The most important boundary in this design is not "manual vs autonomous". It is
"hard constraint vs learnable behavior".

### Hard constraints

These must remain deterministic:

- whether an action class is allowed at all
- whether approval is required
- whether the runtime binding is strong enough
- whether the current channel surface supports the requested path
- whether the source policy allows the acquisition source
- whether topology expansion is permitted

### Learnable behavior

These may improve over time:

- which allowed action to try first
- how to reformulate a search query
- which skill or provider candidate is most promising
- how to allocate bounded budgets
- which strategy performs better under a fixed policy envelope

That separation is what keeps future RL or self-improvement compatible with
governance.

## Decision Contract

The live runtime should expose one deterministic decision contract.

Suggested input shape:

- resolved product profile
- autonomy-policy snapshot
- capability action class
- governance profile
- runtime binding
- channel support facts
- turn budget facts

Suggested output shape:

- `allow`
- `approval_required(reason_code)`
- `deny(reason_code)`

Reason codes should stay explicit and operator-visible.

Example families:

- `product_mode_disallows_capability_acquisition`
- `autonomy_policy_binding_missing`
- `autonomy_policy_channel_support_missing`
- `autonomy_policy_source_policy_denied`
- `autonomy_policy_budget_exceeded`

`product mode` may still own the operator-facing vocabulary, but the internal
reason model should not be limited to mode names alone.

## Why Product Mode Still Matters

This design does not replace `product mode`.

`product mode` is still useful because it gives operators:

- a simple way to choose a safety and autonomy posture
- a clear surface for channel defaults
- understandable explanations in user-facing interfaces

The refinement is internal:

- product mode stays
- autonomy policy becomes the kernel

## Integration Strategy

### Near term

- keep the existing product-mode design as the product-facing contract
- introduce an autonomy-policy kernel underneath it
- compile product-mode presets into policy snapshots
- keep discovery-first intact

### Medium term

- add capability-action classification to the catalog
- add decision evaluation and blocked-reason propagation
- add channel SDK support metadata
- emit policy and outcome telemetry suitable for learning-time analysis

### Long term

- add ranking models or learned strategy selection on top of the policy kernel
- add a governed evolution plane with replay, shadow, and promotion workflows
- connect, but do not merge, memory-pipeline evolution from issue `#455`

## Risks

### Risk: keeping all logic in `product mode`

Rejected because the abstraction becomes overloaded and hard to evolve.

### Risk: letting the learning layer own hard permissions

Rejected because governance and binding rules must stay deterministic.

### Risk: copying HyperAgents-style self-modification into the live turn loop

Rejected because LoongClaw needs explicit promotion, rollback, and operator
trust.

### Risk: pushing autonomy decisions into channel-specific code

Rejected because channels should declare capability facts, not local autonomy
policy.

## Recommended Direction

Treat `product mode` as the external profile surface.

Treat `AutonomyPolicy` as the internal runtime kernel.

Treat learning as a ranking layer constrained by policy.

Treat self-evolution as a separate governed promotion plane.

That is the smallest structurally correct design that stays compatible with the
current LoongClaw runtime while leaving room for future RL and self-improvement
without collapsing governance into prompt behavior or soft heuristics.
