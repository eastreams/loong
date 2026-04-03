# Governance Evidence Gap Analysis

Date: 2026-04-03
Status: Active

## Executive Summary

This document compares LoongClaw with Microsoft's
`agent-governance-toolkit` based on public repository artifacts and turns the
comparison into concrete LoongClaw priorities.

The main conclusion is not that LoongClaw lacks architectural depth.
LoongClaw already has a stronger kernel-first shape than many governance
toolkits:

- strict layered kernel boundaries already exist
- capability, policy, and audit are modeled in core execution paths
- protocol, plugin, pack, and runtime planes already have explicit seams
- the roadmap already tracks most of the right long-horizon concerns

The stronger lesson from `agent-governance-toolkit` is different:

- it packages governance into clearer external evidence
- it isolates identity and trust as a first-class product layer
- it treats reliability and compliance as visible governance surfaces
- it publishes benchmark, threat-model, and OWASP evidence in a way that is
  easy for adopters to evaluate

LoongClaw should preserve its current kernel-first direction while improving
the parts that are still under-expressed or under-productized:

1. governance evidence and operator-verifiable security claims
2. trust and identity as a dedicated runtime plane
3. runtime isolation with clearer hardening and recovery semantics
4. plugin supply-chain trust and provenance
5. agent-SRE style reliability semantics
6. memory governance and provenance

## Scope And Evidence Base

This comparison is intentionally bounded.

It uses:

- LoongClaw repository documents and tracked roadmap items
- public `agent-governance-toolkit` repository documents visible on
  2026-04-03

It does not assume hidden design notes, unreleased code, or undocumented
product behavior.

The external calibration sources used here are:

- AGT repository README
- AGT architecture overview
- AGT threat model
- AGT OWASP mapping
- AGT benchmark summary
- AGT package READMEs for Agent OS, AgentMesh, Agent Runtime, and Agent SRE

## Comparison Matrix

| Area | LoongClaw today | AGT today | Practical lesson for LoongClaw |
|------|-----------------|-----------|--------------------------------|
| Kernel architecture | Strong layered kernel with explicit L0-L9 model, strict crate DAG, and core or extension split | Product-family framing across Agent OS, AgentMesh, Runtime, SRE, Compliance, Marketplace, Lightning | Keep the kernel-first architecture; improve the external packaging of those layers |
| Policy and approvals | Capability-gated design, policy extensions, approval workflows, and structured audit are already kernel-native | Deterministic policy enforcement is central and easy to understand from top-level docs | Preserve LoongClaw's stronger boundary discipline, but present it with simpler adoption-facing evidence |
| Runtime isolation | Shared execution-tier vocabulary and explicit Stage 2 runtime isolation work exist | Runtime rings and kill-switch language are clear and productized | Finish LoongClaw's runtime hardening and make the resulting guarantees legible |
| Identity and trust | Capability and route contracts are present, but agent-to-agent trust is not yet a distinct plane | AgentMesh makes identity, trust, delegation, and protocol bridges first-class | Add a dedicated trust and identity lane instead of leaving it implicit across existing modules |
| Audit and evidence | Security docs, roadmap items, and durable-audit direction exist | Threat model, benchmark tables, OWASP mapping, and compliance narratives are directly published | Promote LoongClaw from internal correctness docs to external evidence packages |
| Plugin ecosystem | Plugin IR, support matrices, signing direction, and trust tiers are planned | Marketplace and supply-chain framing are already externalized | Treat plugin provenance and activation trust as a core adoption surface, not a later ecosystem detail |
| Reliability | Retry, rate shaping, circuit breakers, and adaptive concurrency already exist inside runtime planning | Agent SRE turns reliability into an explicit product surface with SLOs and chaos language | Consolidate existing reliability primitives into a named operator-facing reliability model |
| Memory governance | Memory architecture and profile work exist, but provenance and scope controls are still incomplete | Memory poisoning and context integrity are part of the governance story | Prioritize memory provenance and policy protection before adding richer memory behaviors |
| External adoption surface | Strong docs for architecture and roadmap; fewer benchmark or compliance-facing artifacts | Heavy emphasis on package entry points, framework integration, compliance, benchmarks, and CI signals | Avoid premature multi-language expansion, but improve bridges, proofs, and integration narratives |

## Decision Posture

| Area | Posture | Why |
|------|---------|-----|
| Governance evidence | Borrow | LoongClaw needs clearer threat, benchmark, and control-coverage artifacts without changing its architecture |
| Trust and identity | Adapt | The need is real, but it should land as a kernel-aligned plane rather than a package-family clone |
| Runtime isolation story | Adapt | The product framing is useful, but LoongClaw should finish real runtime constraints instead of only adopting ring language |
| Plugin marketplace and provenance | Borrow | Supply-chain visibility and signing posture are directly aligned with LoongClaw's Stage 4 direction |
| Agent SRE framing | Adapt | Existing reliability primitives should be consolidated, but LoongClaw does not need to mimic AGT's package split or service surface immediately |
| Multi-language SDK breadth | Do not copy yet | LoongClaw's current leverage comes from Rust-kernel depth and protocol bridges, not from immediately matching AGT's language spread |

## What AGT Does Well

### 1. Governance Evidence Is Productized

AGT does not only describe its architecture.
It also publishes:

- benchmark results
- threat-model language
- OWASP risk mapping
- package roles
- framework integration claims
- CI and supply-chain workflow surfaces

That makes the repository easier to evaluate by security, platform, and
adoption stakeholders.

### 2. Identity And Trust Are First-Class

AGT splits trust concerns into a distinct `AgentMesh` layer instead of hiding
them inside generic runtime state.

This gives external users a clear mental model for:

- agent identity
- delegation narrowing
- trust scoring
- inter-agent communication
- credential and revocation semantics

### 3. Reliability Is Framed As Part Of Governance

AGT treats SLOs, circuit breakers, replay, chaos testing, and rollout controls
as governance-adjacent, not as a later operational add-on.

That is a useful design signal for LoongClaw because the runtime already
contains reliability primitives that are stronger than their current external
presentation suggests.

### 4. Compliance Is Presented As Evidence, Not Aspiration

AGT publishes an explicit OWASP mapping and ties controls to named runtime
surfaces.

Even when some controls are still partial, the adoption story remains easier to
audit because the repository exposes what each layer is supposed to cover.

### 5. Packaging And CI Tell A Cohesive Story

AGT's repository front door aligns package structure, benchmark claims,
workflows, and docs.

That reduces the gap between "architecture exists" and "operators can adopt it
with confidence."

## What LoongClaw Already Has

LoongClaw already has several strengths that should not be diluted while
learning from AGT.

### 1. Stronger Kernel-First Structure

LoongClaw has:

- a strict seven-crate workspace
- no internal dependency cycles
- a layered L0-L9 execution model
- explicit core or extension execution planes

That is a better long-term foundation than a flat "toolkit package family"
alone.

### 2. Harder Boundaries Around Execution Authority

LoongClaw's stated contract is that execution routes through kernel capability,
policy, and audit rather than treating those concerns as bolt-on middleware.

That is the right architectural bet for long-term governance.

### 3. Richer Runtime Evolution Surface

The roadmap already includes:

- WASM isolation
- process and browser execution tiers
- protocol route authorization
- connector caller provenance
- plugin translation and activation plans
- runtime capability promotion records

This is not a thin assistant shell.
It is already evolving toward a governed runtime substrate.

### 4. Better Internal Roadmap Specificity

LoongClaw's roadmap is unusually specific about:

- what is already delivered
- what remains
- acceptance criteria
- security and audit implications

That precision is valuable and should remain a differentiator.

## What To Borrow And What Not To Copy

### Borrow

LoongClaw should borrow the following patterns from AGT:

- threat-model and control-matrix documentation as durable repository artifacts
- benchmark publication as governance evidence rather than optional marketing
- a dedicated trust and identity plane instead of scattered trust semantics
- reliability framed as an operator-facing governed runtime surface
- plugin provenance and supply-chain trust treated as adoption-critical, not
  ecosystem polish

### Do Not Copy

LoongClaw should avoid copying the following patterns too early or too
directly:

- multi-language SDK breadth before the Rust core runtime story is hardened
- compliance-style external claims that outrun the current kernel guarantees
- any move that weakens the long-term ambition for stronger governed execution
  lanes
- surface expansion that gets ahead of runtime closure, trust modeling, or
  plugin trust boundaries

## Gaps Worth Closing Next

### 1. Governance Evidence Gap

LoongClaw has strong internal documentation, but it still lacks a compact
external evidence set that answers:

- what risks are explicitly covered
- which guarantees are implemented today
- what latency or throughput overhead governance adds
- how to evaluate adoption readiness

This is the most immediate gap relative to AGT.

### 2. Trust And Identity Gap

LoongClaw has capability contracts, runtime bindings, and route semantics, but
it does not yet expose a dedicated trust plane with clear concepts for:

- agent identity
- delegation scope narrowing
- revocation and trust decay
- channel and connector provenance
- inter-agent or multi-surface trust assertions

### 3. Runtime Hardening Gap

LoongClaw has the right Stage 2 direction, but the operator-facing runtime
story is still incomplete until:

- resource limits are enforced across runtime lanes
- rollback and health-check semantics are standardized
- execution tiers map to reproducible isolation guarantees

### 4. Plugin Supply-Chain Gap

The repository already points toward plugin signing, trust tiers, and
reproducible verification, but those items still sit mostly in future-facing
planning.

This matters because plugins become one of the main trust boundaries as soon as
community adoption grows.

### 5. Reliability Packaging Gap

LoongClaw already contains runtime primitives such as retry policy, rate
limits, circuit breakers, and adaptive concurrency.

The gap is not only missing mechanics.
The gap is that these mechanics are not yet consolidated into an explicit
operator-facing reliability surface.

### 6. Memory Governance Gap

Memory remains an important long-term risk surface.
Current tracked debt still includes incomplete memory scopes, provenance, and
policy-aware deletion.

If LoongClaw grows multi-agent or long-horizon behavior before closing this
gap, it risks building on weak context-governance foundations.

## Recommended Priority Plan

This ordering is intentionally front-loaded toward hardening and evidence
before wider ecosystem expansion.

### P0: Governance Truthfulness

Focus:

- publish a LoongClaw threat model
- publish a first-pass OWASP Agentic mapping or equivalent control matrix
- publish benchmark methodology and current performance baselines
- align security, quality, and roadmap docs with current implementation facts

Why this comes first:

- it turns existing architecture into adoption evidence
- it exposes documentation drift earlier
- it sharpens future roadmap choices using shared control language

Definition of done:

- adopters can answer "what is governed today" from public docs
- major security claims have explicit evidence or explicit limitations
- benchmark methodology is reproducible

### P0: Runtime Isolation Completion

Focus:

- finish resource limits for WASM and process lanes
- standardize rollback-on-failure and post-load health checks
- define operator-visible isolation guarantees per execution tier

Why this stays at the top:

- LoongClaw's kernel-first position depends on real execution boundaries
- plugin hotplug and runtime expansion increase the blast radius of weak
  isolation

Definition of done:

- adversarial isolation tests pass
- tier semantics are documented and reproducible
- rollback behavior is deterministic under injected failure

### P1: Trust And Identity Plane

Focus:

- formalize a dedicated trust and identity layer
- model scoped delegation and provenance across channel, connector, and agent
  boundaries
- define minimal trust decay or revocation semantics

Why this comes next:

- it closes a missing structural plane rather than adding optional polish
- it supports future multi-agent, multi-channel, and connector governance work
- it prevents trust semantics from scattering across unrelated modules

Definition of done:

- identity and delegation have explicit runtime contracts
- connector and agent provenance are inspectable and testable
- trust-bound decisions produce auditable evidence

### P2: Plugin Supply Chain Productization

Focus:

- trust tiers
- signing and provenance
- setup-only metadata
- reproducible artifact verification
- activation and ownership conflict evidence

Why this follows P0 and P1:

- the design direction is already strong
- supply-chain trust becomes more important once adoption and plugin volume rise

Definition of done:

- high-risk unsigned plugins cannot auto-activate
- provenance appears in catalog and audit artifacts
- setup guidance does not require runtime execution

### P3: Reliability Surface Consolidation

Focus:

- group existing retry, circuit-breaker, and concurrency controls into a named
  reliability lane
- define replay and incident-investigation expectations
- tie reliability semantics to pack and connector operations where practical

Why this is worth doing:

- much of the implementation substrate already exists
- clearer reliability framing improves operator understanding and future
  benchmarking

Definition of done:

- runtime reliability controls are documented as one coherent surface
- tests cover failure containment and operator diagnostics
- documentation explains boundaries between policy, runtime, and reliability

### P4: Memory Governance Hardening

Focus:

- memory scopes
- provenance
- policy-aware retention and deletion
- context-poisoning resistance

Why this is not optional:

- long-term memory without provenance creates invisible governance debt
- vertical-agent credibility depends on inspectable context ownership

Definition of done:

- scope and provenance are explicit in runtime contracts
- destructive operations are capability and audit gated
- context assembly explains provenance rather than hiding it

## Roadmap Mapping

This comparison does not require a roadmap rewrite.
It suggests a priority refinement across existing stages.

### Stage 1: Baseline Security And Governance

Raise the relative priority of:

- public governance evidence artifacts
- approval and audit truthfulness
- documentation drift closure

### Stage 2: Safe Hotplug Runtime

Treat this stage as a flagship differentiator.
LoongClaw should finish the runtime-isolation story before expanding its
external surface area too aggressively.

### Stage 3: Autonomous Integration Expansion

Keep the integration work, but couple it more explicitly to:

- trust and provenance
- auditable compatibility claims
- reliability telemetry

### Stage 4: Community Plugin Supply Chain

This stage is strategically validated by the AGT comparison.
It should remain a central roadmap pillar rather than a peripheral ecosystem
task.

### Stage 5: Vertical Pack Productization

This stage becomes stronger if P0 through P4 land first.
Vertical packs should inherit:

- governance evidence
- trust semantics
- runtime hardening
- plugin provenance
- reliability contracts

## Non-Goals And Anti-Duplication Rules

This document should not become:

- a replacement for `ARCHITECTURE.md`
- a duplicate of `docs/SECURITY.md`
- a second `docs/QUALITY_SCORE.md`
- a generic competitive-analysis note detached from implementation

It exists to do one specific job:

- convert external calibration into LoongClaw-specific prioritization

Detailed architecture rules remain in the layered kernel design.
Detailed status remains in the roadmap and quality tracking documents.

## References

### LoongClaw

- [Architecture](../../ARCHITECTURE.md)
- [Roadmap](../ROADMAP.md)
- [Security](../SECURITY.md)
- [Quality Score](../QUALITY_SCORE.md)
- [Layered Kernel Design](layered-kernel-design.md)
- [Plugin Package Manifest Contract](plugin-package-manifest-contract.md)
- [Provider Runtime Roadmap](provider-runtime-roadmap.md)
- [Persistent Kernel Audit Sink Design](../plans/2026-03-15-persistent-audit-sink-design.md)
- [Autonomy Policy Kernel Architecture](../plans/2026-03-26-autonomy-policy-kernel-architecture.md)
- [LoongClaw Memory Architecture Design](../plans/2026-03-11-loongclaw-memory-architecture-design.md)

### External Calibration

- [Agent Governance Toolkit README](https://github.com/microsoft/agent-governance-toolkit/blob/main/README.md)
- [Agent Governance Toolkit Architecture](https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/ARCHITECTURE.md)
- [Agent Governance Toolkit Threat Model](https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/THREAT_MODEL.md)
- [Agent Governance Toolkit OWASP Mapping](https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/OWASP-COMPLIANCE.md)
- [Agent Governance Toolkit Benchmarks](https://github.com/microsoft/agent-governance-toolkit/blob/main/BENCHMARKS.md)
- [Agent OS README](https://github.com/microsoft/agent-governance-toolkit/blob/main/packages/agent-os/README.md)
- [AgentMesh README](https://github.com/microsoft/agent-governance-toolkit/blob/main/packages/agent-mesh/README.md)
- [AgentMesh Runtime README](https://github.com/microsoft/agent-governance-toolkit/blob/main/packages/agent-runtime/README.md)
- [Agent SRE README](https://github.com/microsoft/agent-governance-toolkit/blob/main/packages/agent-sre/README.md)
