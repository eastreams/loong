# LoongClaw Architecture

LoongClaw is structured as a 7-crate Rust workspace with a strict acyclic dependency graph.
The kernel enforces layered execution planes separating contracts, security, execution, and
orchestration concerns. This document gives newcomers a high-level map; for the full
specification see [Layered Kernel Design](docs/design-docs/layered-kernel-design.md).

## Crate Structure

```text
contracts (leaf -- zero internal deps)
├── kernel --> contracts
├── protocol (independent leaf)
├── app --> contracts, kernel
├── spec --> contracts, kernel, protocol
├── bench --> contracts, kernel, spec
└── daemon (binary) --> all of the above
```

No dependency cycles. This is non-negotiable.

| Crate | Role |
|-------|------|
| `contracts` | Shared types, capability model, and route model. Zero internal dependencies -- the stable ABI surface. |
| `kernel` | Policy engine, audit timeline, capability token lifecycle, plugin system, integration catalog, and pack boundary enforcement. |
| `protocol` | Transport contracts, typed method routing, json-line stream transport, and linked in-memory channel primitive. Independent leaf crate. |
| `app` | Providers, tools, channels, memory backends, configuration, and conversation engine. Houses all feature-flagged modules. |
| `spec` | Execution specification runner for deterministic test scenarios. |
| `bench` | Performance benchmark harness and gate enforcement. |
| `daemon` | CLI binary (`loongclaw`). Wires all crates into runnable commands: `setup`, `onboard`, `doctor`, `chat`, `run-spec`, benchmarks. |

## Layered Execution Model

The kernel uses a layered model where each layer has clear responsibilities and strict
boundaries. Higher layers depend on lower layers but never the reverse.

### L0 -- Contract Layer

The stable kernel ABI surface. Defines request/outcome structs, the capability model, and
the route model. Backward compatibility is enforced: no breaking changes to public types.
Serialization format and field behavior are part of the kernel contract.

**Key files:** `contracts.rs`

### L1 -- Security and Governance

Every external action must pass through L1. The policy engine evaluates tool calls before
dispatch: each decision combines model intent with a deterministic policy check. Policy
extensions can only tighten behavior, never weaken core policy.

- Capability token issue/revoke/authorize lifecycle
- Human approval gates (per-call or one-time full-access, configurable)
- Plugin security scanning with `block_on_high` hard gate
- External profile integrity: checksum pinning + ed25519 signature verification
- JSONL SIEM export lane with optional fail-closed mode
- Denylist takes highest precedence over all grants

**Key files:** `policy.rs`, `policy_ext.rs`

### L2 -- Execution Planes

Four parallel planes, each following the Core/Extension adapter pattern:

| Plane | Core Adapter | Extension Adapter |
|-------|-------------|-------------------|
| Runtime | Minimal trusted substrate | Rich behavior (WASM, process bridges) |
| Tool | Built-in tools (`shell.exec`, `file.read`, `file.write`) | Community tool adapters |
| Memory | Base storage (SQLite) | Semantic retrieval enrichment |
| Connector | Direct HTTP/protocol calls | Third-party integration adapters |

Extension path never bypasses core path. Core interfaces remain stable and minimal.
Each plane supports explicit default-core selection for deterministic orchestration.

**Key files:** `runtime.rs`, `tool.rs`, `memory.rs`, `connector.rs`

### L3 -- Orchestration

Routes task execution to harness adapters, enforces pack boundaries and capability
boundaries, and bridges L1 policy decisions to L2 execution. The orchestrator is
policy-aware but business-logic-light.

**Key files:** `harness.rs`, `kernel.rs`

### Higher Layers

| Layer | Scope |
|-------|-------|
| L4 -- Observability | Audit timeline, sink abstraction, deterministic clocking for reproducible tests |
| L5 -- Vertical Packs | Domain packaging contract (`pack_id`, version, capabilities, allowed connectors) |
| L5.5 -- Protocol Foundation | Transport frame contracts, typed route resolution, bounded channel primitives |
| L6 -- Integration Control | Autonomous provider/channel provisioning, plugin scanning, hotplug/hotfix workflows |
| L7 -- Plugin Translation | Multi-language plugin IR, bridge-kind inference, activation plan generation |
| L8 -- Self-Awareness | Codebase snapshots, architecture guard policy, immutable-core mutation protection |
| L9 -- Bootstrap | Plugin activation lifecycle (apply/defer/skip), policy-bounded bootstrap execution |

## Design Principles

These are the core principles for anyone working in this codebase. They are enforced
mechanically where possible.

1. **Kernel-first** -- all execution paths route through the kernel's capability, policy, and audit system. No shadow paths.
2. **No breaking changes** -- new features are additive only. Existing public API signatures stay unchanged.
3. **Capability-gated by default** -- every tool call, memory operation, and connector invocation requires a valid `CapabilityToken`.
4. **Audit everything security-critical** -- policy denials, token lifecycle events, and module invocations all emit structured audit events.
5. **7-crate DAG, no cycles** -- dependency direction is non-negotiable.
6. **Tests first** -- if a behavior isn't tested, it doesn't exist. All tests pass at every commit.
7. **Proven technology preferred** -- choose well-understood, composable dependencies over opaque packages.
8. **Repository is the system of record** -- design decisions and architectural context live in `docs/`, not in chat threads.
9. **Automate first** -- prefer linters, CI gates, and pre-commit hooks over code review comments.
10. **Strictly avoid over-engineering** -- the minimum complexity for the current task is the right amount.

## Further Reading

| Topic | Document |
|-------|----------|
| Full layer specification (L0-L9) | [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) |
| Harness engineering & backpressure | [Harness Engineering](docs/design-docs/harness-engineering.md) |
| Design decisions, patterns & catalog | [Design Docs Index](docs/design-docs/index.md) |
| Security model & gaps | [Security](docs/SECURITY.md) |
| Stage-based roadmap | [Roadmap](docs/ROADMAP.md) |
| Build and kernel invariants | [Reliability](docs/RELIABILITY.md) |
| Domain quality grades | [Quality Score](docs/QUALITY_SCORE.md) |
| Product principles | [Product Sense](docs/PRODUCT_SENSE.md) |
| Contributor workflow and recipes | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Examples and spec files | [Examples](examples/README.md) |
