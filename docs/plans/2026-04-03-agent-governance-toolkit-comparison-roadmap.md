# Agent Governance Toolkit Comparison And Roadmap

Related:

- issue `#843`
- existing epics and RFCs: `#196`, `#420`, `#440`, `#455`, `#766`, `#831`

## Purpose

This document turns a comparison between LoongClaw and
`microsoft/agent-governance-toolkit` into a repo-scoped roadmap artifact.

The goal is not to copy AGT.

The goal is to:

1. identify which AGT patterns LoongClaw should borrow
2. identify which patterns LoongClaw should adapt to its kernel-first design
3. identify which patterns LoongClaw should explicitly avoid copying
4. turn those conclusions into a prioritized backlog direction

## Comparison Baseline

### AGT profile

As of `2026-04-03`, AGT presents itself as a multi-package governance toolkit
with:

- policy enforcement
- zero-trust identity
- execution sandboxing
- agent SRE
- plugin marketplace
- compliance and OWASP framing
- multi-language SDKs and many framework integrations

Primary external references:

- `https://github.com/microsoft/agent-governance-toolkit`
- `https://github.com/microsoft/agent-governance-toolkit/blob/main/README.md`
- `https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/ARCHITECTURE.md`
- `https://github.com/microsoft/agent-governance-toolkit/blob/main/BENCHMARKS.md`
- `https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/OWASP-COMPLIANCE.md`
- `https://github.com/microsoft/agent-governance-toolkit/blob/main/SECURITY.md`

Two AGT traits matter most for this comparison:

1. it is productized around governance language, compliance language, and
   ecosystem surface area
2. it explicitly documents that its enforcement boundary is application-layer,
   not OS-kernel isolation

### LoongClaw profile

LoongClaw is already a governed runtime foundation with stronger internal
boundary discipline than a typical middleware-only governance wrapper.

Concrete proof points already exist in the repository:

- strict 7-crate DAG and kernel layer model in `ARCHITECTURE.md` and
  `docs/design-docs/layered-kernel-design.md`
- explicit kernel-first invariants in `docs/design-docs/core-beliefs.md`
- policy-gated runtime context bootstrap in `crates/app/src/context.rs`
- explicit protocol router and transport contracts in
  `crates/protocol/src/lib.rs`
- explicit runtime binding seams in
  `crates/app/src/conversation/runtime_binding.rs` and
  `crates/app/src/provider/runtime_binding.rs`
- governed dry-run promotion and evidence artifacts in
  `docs/plans/2026-03-18-runtime-capability-index-readiness-gate-design.md`
  and `docs/plans/2026-03-20-runtime-capability-delta-evidence-design.md`
- benchmark harness and benchmark workflow surfaces in `crates/bench/*`,
  `examples/benchmarks/*`, and `.github/workflows/perf-benchmark.yml`

## Executive Summary

LoongClaw should not try to imitate AGT's breadth-first platform shape.

AGT's best lessons are mostly above the kernel:

- governance productization
- identity and trust articulation
- SRE framing
- compliance evidence packaging
- ecosystem intake language

LoongClaw's strongest advantage is below that layer:

- stronger kernel boundary discipline
- more explicit governed execution seams
- better long-term artifact thinking around capability promotion and runtime
  evidence
- a more defensible path to hard runtime policy than prompt-level governance

The right direction is therefore:

- keep LoongClaw's kernel-first architecture
- absorb AGT's operator-facing governance packaging
- strengthen the missing runtime and evidence layers before expanding ecosystem
  breadth

## What LoongClaw Already Does Well

### 1. Kernel and protocol boundaries are structurally stronger

LoongClaw already enforces a strict workspace DAG and publishes its layer model
as a hard contract:

- `ARCHITECTURE.md`
- `docs/design-docs/core-beliefs.md`
- `docs/design-docs/layered-kernel-design.md`
- `crates/protocol/src/lib.rs`

This is a real implementation seam, not just a packaging story.

### 2. Runtime authority is explicit instead of implicit

The conversation and provider paths now model authority with explicit runtime
binding types:

- `crates/app/src/conversation/runtime_binding.rs`
- `crates/app/src/provider/runtime_binding.rs`
- `docs/SECURITY.md`

That is a better base for future autonomy control than informal prompt policy.

### 3. Governance artifacts already exist as first-class objects

LoongClaw already treats runtime promotion and evidence as governed artifacts:

- `docs/plans/2026-03-18-runtime-capability-index-readiness-gate-design.md`
- `docs/plans/2026-03-20-runtime-capability-delta-evidence-design.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`

This is one of LoongClaw's strongest long-term differentiators.

### 4. Runtime controls are already appearing in real execution paths

LoongClaw already implements concrete runtime control surfaces for some lanes:

- plugin security scan findings and correlation ids in
  `crates/spec/src/spec_execution/security_scan_eval.rs`
- bridge runtime policy and execution tiers in
  `crates/spec/src/spec_runtime.rs`
- adaptive rate limiting, circuit breakers, and concurrency control in
  `crates/spec/src/programmatic.rs`
- verification coverage in `crates/daemon/tests/integration/programmatic.rs`

This matters because AGT's SRE and governance story is attractive only if it
maps to real control paths. LoongClaw already has the start of those paths.

## Area-By-Area Comparison

### 1. Identity and trust

AGT strength:

- AGT productizes identity as a dedicated trust layer with cryptographic agent
  identity, trust scores, delegation language, and inter-agent trust framing.

LoongClaw current state:

- LoongClaw has explicit runtime-self and resolved-identity lanes for prompt and
  session continuity:
  - `crates/app/src/runtime_self.rs`
  - `docs/product-specs/runtime-self-continuity.md`
  - `docs/plans/2026-03-24-runtime-self-advisory-boundary-design.md`
- LoongClaw does not yet expose a dedicated workload identity or trust plane
  comparable to AGT's productized agent identity framing.
- Ed25519 currently appears in security profile signing and related runtime
  policy surfaces rather than as a general agent identity plane:
  - `docs/ROADMAP.md`
  - `crates/spec/src/spec_runtime.rs`

Lesson:

- `adapt`

Why:

- LoongClaw should learn from AGT's identity and trust articulation.
- LoongClaw should not let soft trust scores become the source of hard runtime
  authority.
- capability tokens and kernel policy should remain the hard permission root.

Recommended direction:

- introduce a dedicated identity and trust plane for:
  - operator-visible workload identity
  - delegation provenance
  - trust decay and trust hints
  - inter-agent or cross-surface attribution
- keep that plane subordinate to kernel capability and policy enforcement

### 2. Audit integrity and tamper evidence

AGT strength:

- AGT productizes append-only and tamper-evident audit language aggressively.

LoongClaw current state:

- LoongClaw already has explicit audit sink contracts and durable JSONL support:
  - `crates/kernel/src/audit.rs`
  - `crates/app/src/context.rs`
  - `docs/RELIABILITY.md`
- LoongClaw still documents a real tamper-evidence gap:
  - `docs/SECURITY.md` explicitly says there is no HMAC chain
- audit defaults and harness-specific exceptions are already being tightened:
  - `docs/plans/2026-03-18-retire-noop-audit-default-design.md`
  - `docs/plans/2026-03-18-spec-audit-contract-convergence-design.md`

Lesson:

- `borrow`

Why:

- AGT is right that governance claims are much stronger when audit evidence is
  operator-verifiable.
- LoongClaw already has the sink architecture to support this cleanly.

Recommended direction:

- add hash-chained or similarly tamper-evident audit journaling
- add a verification and export surface for audit integrity
- keep the sink contract small and additive

### 3. Runtime isolation and resource control

AGT strength:

- AGT makes runtime isolation a first-class product story through execution
  rings and termination control.

LoongClaw current state:

- LoongClaw already models shared execution tiers across process, browser, and
  WASM lanes:
  - `docs/SECURITY.md`
  - `crates/spec/src/spec_runtime.rs`
- LoongClaw already enforces some bridge and WASM safety controls:
  - process allowlists
  - timeout controls
  - WASM path constraints
  - size caps
  - fuel limits
  - strict protocol checks
- concrete remaining gaps are already documented in `docs/ROADMAP.md`:
  - CPU budget refinement
  - memory limits
  - timeout and termination policy completion
  - rollback-on-failure and health checks

Lesson:

- `adapt`

Why:

- LoongClaw should learn from AGT's runtime-control product language.
- LoongClaw should not copy the metaphor if the enforcement contract stays
  weaker than the name suggests.

Recommended direction:

- continue the current execution-tier path
- convert tier vocabulary into stronger enforced budgets and termination
  semantics
- keep evidence surfaces explicit so runtime claims remain truthful

### 4. SRE, circuit breakers, and budgeting

AGT strength:

- AGT exposes agent SRE as a dedicated product pillar with SLOs, error budgets,
  circuit breakers, replay, and cost language.

LoongClaw current state:

- LoongClaw already has circuit-breaker and concurrency machinery in the
  programmatic execution path:
  - `crates/spec/src/programmatic.rs`
  - `crates/daemon/tests/integration/programmatic.rs`
- LoongClaw also has benchmark and comparison infrastructure:
  - `crates/bench/src/lib.rs`
  - `crates/bench/tests/bench_integration.rs`
  - `examples/benchmarks/programmatic-pressure-*.json`
  - `.github/workflows/perf-benchmark.yml`
- what is still missing is a unified runtime-wide SRE plane:
  - no repo-wide SLO contract
  - no unified error-budget contract
  - no cost-guard plane
  - no single replay and chaos layer framed as a runtime subsystem

Lesson:

- `borrow`

Why:

- the AGT SRE framing is valuable
- LoongClaw already has enough runtime control primitives to justify elevating
  them into a coherent plane

Recommended direction:

- promote existing circuit-breaker, rate-shaping, and benchmark evidence into a
  runtime-wide SRE model
- keep the first slice narrow:
  - connector and provider budgets
  - runtime-level circuit breaker visibility
  - benchmark-backed regression gates

### 5. Plugin supply chain and provenance

AGT strength:

- AGT productizes marketplace, signing, provenance, and supply-chain governance
  as external-facing ecosystem language.

LoongClaw current state:

- LoongClaw already has a stronger kernel-governed plugin intake story than a
  simple marketplace wrapper:
  - manifest and translation path in `crates/kernel/src/plugin.rs` and
    `crates/kernel/src/plugin_ir.rs`
  - bootstrap policy in `crates/kernel/src/bootstrap.rs`
  - manifest-first contract work in
    `docs/design-docs/plugin-package-manifest-contract.md`
  - security scan evidence and blocking in
    `crates/spec/src/spec_execution/security_scan_eval.rs`
- the roadmap already names the missing ecosystem layers:
  - signing metadata
  - trust tiers
  - provenance visibility
  - reproducible verification
  - setup-only metadata
  - ownership conflict handling
  - `docs/ROADMAP.md`

Lesson:

- `borrow`

Why:

- AGT is right that plugin and supply-chain governance must be operator-visible.
- LoongClaw already has the structural seams to do this without weakening its
  runtime boundary.

Recommended direction:

- complete manifest-first package intake
- add signing and provenance as first-class plugin catalog data
- keep untrusted community extensions on controlled execution lanes by default

### 6. Governance and compliance artifacts

AGT strength:

- AGT makes governance visible through OWASP mapping, benchmark claims, security
  tooling, and regulator-facing language.

LoongClaw current state:

- LoongClaw already has serious internal governance artifacts:
  - `docs/ROADMAP.md`
  - `docs/RELIABILITY.md`
  - `docs/SECURITY.md`
  - `docs/QUALITY_SCORE.md`
  - architecture drift reporting in `docs/releases/architecture-drift-2026-03.md`
  - runtime capability evidence and planning docs under `docs/plans/*`
- what LoongClaw does not yet have is a productized public governance evidence
  layer comparable to AGT's:
  - no explicit OWASP mapping
  - no public compliance scorecard
  - no single `verify governance` style operator-facing report

Lesson:

- `adapt`

Why:

- LoongClaw should absolutely improve operator-visible governance evidence.
- LoongClaw should not make public coverage claims before the underlying
  evidence exports are strong enough.

Recommended direction:

- treat compliance and governance mapping as an evidence-export layer above the
  current kernel and audit work
- ship it after audit integrity, runtime policy evidence, and plugin provenance
  become stronger

## What LoongClaw Should Not Copy

### 1. Breadth-first ecosystem sprawl

AGT's package count, SDK count, and integration count are useful for adoption,
but that shape would be premature for LoongClaw.

LoongClaw should not prioritize:

- many SDKs before the internal governance core is complete
- framework-count optics over runtime-boundary quality
- marketplace breadth before provenance and trust-tier contracts land

### 2. Security language that outruns enforcement

AGT correctly discloses that it is application-layer governance.

LoongClaw should preserve the same honesty:

- do not claim hard isolation where only advisory or soft policy exists
- do not rename partial controls into stronger metaphors than the code supports
- do not publish broad compliance coverage without exportable evidence

### 3. Soft trust as hard authorization

Trust scoring is useful as a routing and risk signal.

It should not become the root authority for:

- tool access
- runtime mutation
- topology expansion
- bootstrap policy bypass

LoongClaw should keep kernel capability and policy as the hard control plane.

## Prioritized Roadmap

### Near-term hardening

Target window:

- next 30 days

Priority outcomes:

1. finish tamper-evident audit integrity work
2. land the autonomy-policy kernel and product-mode control plane
3. turn execution-tier language into stronger enforced runtime budgets

Concrete directions:

- add audit-chain verification on top of `crates/kernel/src/audit.rs`
- close the remaining design-to-implementation gap from:
  - `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`
  - `docs/plans/2026-03-26-product-mode-capability-acquisition-design.md`
- prioritize `docs/ROADMAP.md` Stage 2 items that make execution tiers more
  enforceable

Why first:

- these are foundational truthfulness and control-plane tasks
- they improve nearly every later governance claim

### Medium-term runtime and governance improvements

Target window:

- next 31 to 60 days

Priority outcomes:

1. add a dedicated identity and trust plane
2. elevate existing runtime controls into a unified SRE surface
3. complete plugin provenance and trust-tier contracts

Concrete directions:

- define a workload identity and delegation provenance model that stays
  subordinate to kernel authority
- unify circuit-breaker, rate-shaping, retry, and benchmark evidence under one
  SRE framing
- drive plugin work through:
  - `docs/design-docs/plugin-package-manifest-contract.md`
  - `docs/ROADMAP.md` Stage 4

Why second:

- these layers are valuable, but they depend on the near-term hardening work to
  avoid becoming mostly product language

### Later ecosystem and productization work

Target window:

- next 61 to 90 days

Priority outcomes:

1. expose a clearer operator-facing governance verification surface
2. add selective external integration and SDK packaging
3. publish governance evidence without overstating guarantees

Concrete directions:

- add a governance verification report surface on top of existing artifact and
  audit contracts
- add carefully scoped middleware or SDK entrypoints only after the core
  evidence contracts are stable
- consider OWASP or similar mapping only when the evidence-export surface is
  mature enough to back every claim

Why later:

- this is where AGT's strongest productization lessons live
- LoongClaw should reach this layer after core governance truthfulness improves,
  not before

## Backlog Mapping

### Existing work this roadmap should inform

- `#196` security gap tracking
- `#420` production trust hardening
- `#440` runtime self model and continuity
- `#455` governed self-evolution and memory pipeline
- `#766` binding-first seam tightening
- `#831` source-aware external skills intake and security scan expansion

### New backlog candidates to open after this doc lands

1. Add tamper-evident audit chain verification and export tooling
2. Add a dedicated identity and trust plane for workload attribution and
   delegation provenance
3. Promote runtime circuit-breaker, budget, and benchmark controls into a
   unified SRE surface
4. Add plugin signing, provenance, and trust-tier visibility to the runtime
   catalog
5. Add an operator-facing governance verification report built on existing
   evidence artifacts

## Recommendation

The right strategic move is not to turn LoongClaw into a Rust clone of AGT.

The right move is:

- keep LoongClaw's stronger kernel-first runtime path
- borrow AGT's governance productization where it strengthens operator clarity
- refuse shortcuts that trade architectural truth for ecosystem optics

If LoongClaw follows that path, it can become both:

- a more rigorous governed runtime foundation for its chosen scope
- a more legible governance product than it is today
