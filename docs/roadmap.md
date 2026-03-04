# ChumOS Roadmap

Last updated: 2026-03-04

This roadmap is execution-focused. Every stage has:

- concrete deliverables
- explicit security and stability gates
- test and CI acceptance criteria

## North Star

Build a layered Agentic OS kernel that is:

- minimal at the core
- strong at policy and safety boundaries
- deeply integrable in both directions (others integrate ChumOS, ChumOS integrates others)
- hot-pluggable and community-extensible without core mutation
- customizable into vertical domain systems through declarative packs

## Architecture Invariants

1. Core contracts stay small and stable.
2. Rich behavior lands in extension planes, plugin packs, and connectors.
3. High-risk actions require human authorization under policy.
4. Untrusted plugins default to strict scan + restricted runtime.
5. Every security-critical decision must be auditable.

## Stage 0: Kernel Contract Freeze (Done)

Status: complete  
Focus: create minimal but strong core boundaries.

Delivered:

- core/extension split for `runtime`, `tool`, `memory`, `connector`
- capability-based policy engine and policy extension chain
- deterministic audit timeline and event sink abstraction
- vertical pack capability/connectors boundary model

Exit criteria met:

- core boundaries enforced by unit tests
- deterministic audit schema tests passing
- property tests for capability boundary invariants passing

## Stage 1: Baseline Security & Governance (In Progress)

Status: in progress  
Focus: medium-balanced defaults + hard stops for high-risk behavior.

Delivered:

- medium-balanced human approval model with:
  - per-call allow mode
  - one-time full-access mode
  - denylist precedence
  - external risk profile loading
- plugin bridge support matrix and checksum integrity lock
- plugin security scan with `block_on_high`
- WASM static scan controls:
  - allowed artifact paths
  - module size cap
  - digest pin support
  - import policy (`allow_wasi`, blocked prefixes)
- typed audit emission for security scan summary (`SecurityScanEvaluated`)

Remaining:

- sign/lock external security profile integrity (profile checksum pinning)
- enrich audit stream with per-finding correlation IDs for SIEM pipelines

Exit criteria:

- all high-risk tool calls blocked without explicit approval
- blocked community plugin paths demonstrably fail closed
- security scan and approval decisions both visible in structured audit

## Stage 2: Safe Hotplug Runtime (Next)

Status: planned  
Focus: runtime-grade isolation for untrusted extension execution.

Planned deliverables:

- WASM runtime lane with enforced resource limits:
  - CPU budget
  - memory limits
  - timeout/termination policy
- process bridge sandbox profile tiers (`restricted`, `balanced`, `trusted`)
- hot-reload lifecycle hooks:
  - pre-load validation
  - rollback-on-failure
  - post-load health check

Acceptance criteria:

- runtime isolation e2e tests (normal + adversarial cases) passing
- deterministic rollback behavior validated under injected failures
- no core-path mutation allowed by plugin hotplug workflow

## Stage 3: Autonomous Integration Expansion (Next)

Status: planned  
Focus: dynamic provider/channel integration without hardcoding.

Planned deliverables:

- connector contract versioning and compatibility matrix
- provider/channel auto-provision enhancements:
  - missing connector discovery strategy
  - deterministic configuration synthesis
  - idempotent reconciliation
- bi-directional protocol bridge adapters:
  - OpenAI-compatible
  - Anthropic-compatible
  - MCP server/client lanes

Acceptance criteria:

- repeated auto-provision runs are idempotent
- unsupported protocol paths fail with explicit typed reasons
- full integration catalog diff is auditable and reversible

## Stage 4: Community Plugin Supply Chain (Next)

Status: planned  
Focus: open ecosystem without sacrificing trust boundaries.

Planned deliverables:

- multi-language plugin intake pipeline:
  - manifest extraction
  - bridge inference
  - safety classification
- plugin packaging and signing metadata
- trust policy tiers (`official`, `verified-community`, `unverified`)
- reproducible plugin artifact verification in CI

Acceptance criteria:

- unsigned/untrusted high-risk plugins never auto-activate
- plugin provenance visible in catalog and audit events
- plugin translation + activation plans deterministic across runs

## Stage 5: Vertical Pack Productization (Next)

Status: planned  
Focus: 15-minute vertical customization workflow.

Planned deliverables:

- pack template generator:
  - domain prompt baseline
  - tool/connector policy presets
  - evaluation set bootstrap
- pack-level SLO/quality dashboard:
  - latency
  - success ratio
  - safety violations
- guided hardening checklist per vertical pack

Acceptance criteria:

- new vertical pack reaches runnable state in <= 15 minutes
- pack policy and required capabilities fully declarative
- regression pack tests can be generated and executed automatically

## Quality Gate Matrix (Always On)

All roadmap stages must keep these gates green:

1. `cargo fmt`
2. `cargo test` (workspace full pass)
3. Security regression set (approval, scan, bridge constraints)
4. Audit schema stability checks for critical event kinds
5. No hardcoded risk exceptions when config-driven alternatives exist

## Current Priority Order

1. Stage 1 completion: profile integrity locking + audit correlation enrichment
2. Stage 2 kickoff: WASM runtime isolation lane
3. Stage 3 baseline: connector contract versioning and idempotent reconciliation

