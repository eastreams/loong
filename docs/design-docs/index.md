# Design Documents Index

Catalog of design documents and architectural decisions.

## Active Design Documents

| Document | Scope | Status |
|----------|-------|--------|
| [Core Beliefs](core-beliefs.md) | Engineering principles and taste enforcement | Living |
| [Local Product Control Plane](local-product-control-plane.md) | Localhost-only platform layer above the runtime and below future HTTP/Web UI surfaces | Active |
| [Discovery-First Tool Runtime Contract](discovery-first-tool-runtime-contract.md) | Provider-core tools, leases, parser rewrites, and follow-up turn contract | Active |
| [Governance Simplification Classification](governance-simplification-classification.md) | Classifies governance surfaces as structural, transitional, cleanup-safe, or replacement-first | Active |
| [Layered Kernel Design](layered-kernel-design.md) | L0-L9 kernel layer specification and boundary rules | Living |
| [Plugin Package Manifest Contract](plugin-package-manifest-contract.md) | Manifest-first plugin metadata, setup surface, and slot ownership contract | Active |
| [Provider Runtime Roadmap](provider-runtime-roadmap.md) | Provider/runtime evolution strategy | Active |
| [ACP/ACPX Pre-Embed](acp-acpx-preembed.md) | Advanced cryptographic primitives | Active |
| [Harness Engineering](harness-engineering.md) | Environment design for agent-driven development | Active |

## Key Patterns

| Pattern | Description | Where Enforced |
|---------|-------------|----------------|
| Core/Extension split | Every execution plane has a core adapter and optional extensions | `kernel/src/tool.rs`, `runtime.rs`, `memory.rs`, `connector.rs` |
| Capability-gated access | Every resource access requires an explicit capability token | `kernel/src/policy.rs` |
| Rule of Two | Tool calls require both LLM intent and deterministic policy approval | `kernel/src/policy.rs` |
| Registry pattern | Adapters registered by name into `BTreeMap<String, Arc<dyn Trait>>` | All execution planes |
| Generation-based revocation | `AtomicU64` threshold invalidates all tokens with generation <= N | `kernel/src/kernel.rs` |
| Policy extension chain | Chain-of-responsibility: multiple extensions evaluated in order, any can deny | `kernel/src/policy_ext.rs` |

## Tracked Deviations

None currently.

## Decision Log

All decisions from the research repository. Status reflects implementation reality, not aspiration.

### Cross-Domain

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-001 | Zircon-style capability model (handle + rights, membrane revocation) | Partial — tokens enforced, membrane not checked (TD-003) |
| D-002 | Hybrid agent lifecycle (Actix states + Erlang/OTP supervision) | Stub — lifecycle struct exists, no supervision |
| D-015 | OAuth 2.1 external + capability internal auth | Research — not wired |

### Runtime Core (Domain 01)

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-003 | Append-only event log as single source of truth | Partial — audit events exist, in-memory only (TD-006) |
| D-004 | Materialized views from event log | Not started |
| D-005 | Control/data plane separation | Partial — planes exist, not fully separated |
| D-006 | Consensus-agnostic event log trait | Not started — no trait defined |
| D-007 | Structured events using tracing crate | Not started |
| D-008 | Work-stealing scheduler with fuel budgets | Not started |

### Academic & Practice (Domains 09-10)

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-009 | Single-threaded Tokio-based async agent loop | Implemented |
| D-010 | Kernel minimality principle | Implemented — core beliefs #10 |

### Protocols (Domain 04)

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-011 | Multi-protocol pluggable transports | Partial — protocol crate exists |
| D-012 | JSON-RPC external, binary internal | Partial — JSON-line transport implemented |
| D-013 | Three-tier capability negotiation | Research |
| D-014 | Algebraic protocol versioning | Research |

### Memory (Domain 05)

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-016 | MemoryStore trait (4 typed async methods) | Not started — using string dispatch (TD-008) |
| D-017 | MemoryScope enum (Task, Session, Agent, Global) | Not started — flat session_id (TD-010) |
| D-018 | SQLite + FTS5 default backend (WAL, feature-gated sqlite-vec) | Partial — SQLite turns table, no FTS5 (TD-011) |
| D-019 | Mandatory provenance fields (10 fields: UUID, trust_tier, hash, agent, TTL...) | Not started |
| D-020 | Configurable trust scoring (Tier 0-3) | Not started |
| D-021 | Blake3 content hashing (feature-gated `pure` mode) | Not started |
| D-022 | Capability-scoped deletion (tombstone audit trail) | Not started |

### WASM Plugins (Domain 08)

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-023 | WebAssembly Component Model for all plugins | Research (v0.2) |
| D-024 | Wasmtime as runtime engine | Partial — wasmtime integrated |
| D-025 | Per-invocation plugin isolation | Research |
| D-026 | OCI artifact distribution | Research |
| D-027 | WIT plugin contracts | Research |
| D-028 | WASI 0.3 async target | Research |
| D-029 | Epoch interruption for production, fuel metering for testing | Not started (TD-013) |
| D-030 | Zero-capability-default WASI injection | Research |
