# Loong Architecture

Loong is structured as an 8-crate Rust workspace with a strict acyclic
dependency graph. The kernel enforces layered execution planes separating
contracts, security, execution, and orchestration concerns.

This file describes the architecture as it is currently governed in the
repository. The crate split, layer names, and ownership map are deliberate
decisions, but they are not presented as eternal truths. If the product shape
changes, maintainers may revise this map explicitly through design work rather
than by letting boundary drift accumulate accidentally.

Public reader-facing architecture docs live under `site/`. This file remains
the repository-native architecture map for contributors, source readers, and
maintainers who need the codebase-level structure behind the Mintlify docs.

## Route By Audience

| If you are trying to... | Start here |
| --- | --- |
| read the public builder-facing architecture summary first | [site/build-on-loong/architecture.mdx](site/build-on-loong/architecture.mdx) |
| understand the crate DAG and where changes usually belong in the repo | this file |
| inspect the full layer specification | [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) |
| understand the broader repository docs layering | [docs/README.md](docs/README.md) |

## Read This File When

- you need the repository-native architecture map rather than the shorter public
  docs summary
- you are deciding which crate or layer should own a change
- you are reviewing whether a contribution introduces boundary drift or hidden
  sidecar execution
- you need the codebase-level map before opening the deeper design docs

## Section Map

| Section | Read it when... |
| --- | --- |
| [Crate Structure](#crate-structure) | you need the direct DAG and the higher-level ownership split across crates |
| [Practical Ownership Map](#practical-ownership-map) | you need the shortest source-driven explanation of what each crate family really owns |
| [Layered Execution Model](#layered-execution-model) | you need the architecture layers and what each one protects |
| [Design Principles](#design-principles) | you need the invariants that should survive implementation detail changes |
| [Further Reading](#further-reading) | you need the deeper source docs behind one area |

## Crate Structure

The workspace DAG matters, but so does the ownership model behind it. The code
currently splits into one stable contract crate, one governed execution core,
one product/runtime crate, two validation rails, and one daemon assembly crate.

```text
direct dependency DAG

contracts      (stable contract vocabulary)
├── kernel        -> contracts
├── protocol      (independent transport foundation)
├── bridge-runtime -> contracts, kernel, protocol
├── app           -> contracts, kernel
├── spec          -> contracts, kernel, protocol, bridge-runtime
├── bench         -> kernel, spec
└── daemon        -> app, bench, contracts, kernel, spec, bridge-runtime
```

No dependency cycles. This is non-negotiable.

## Practical Ownership Map

```text
contracts  stable vocabulary for capability, policy, audit, runtime, tool, and memory contracts
kernel     governed execution core: policy, audit, planes, harness, plugin/integration control
protocol   transport and route foundation used by the spec rail
app        product/runtime layer: providers, channels, tools, memory, conversation, presentation
spec       deterministic execution rail and bootstrap/test runtime
bench      performance and pressure rail on top of spec/kernel
daemon     operator CLI and service assembly over the lower layers
```

| Crate | Role |
|-------|------|
| `contracts` | Shared types and stable contract vocabulary: capability tokens, policy/audit types, runtime/tool/memory request-outcome shapes, task state, namespaces, and pack manifests. Zero internal dependencies. |
| `kernel` | Governed execution core. Owns audit, policy, runtime/tool/memory/connector planes, harness brokerage, task supervision, plugin and integration control, bootstrap execution, and architecture awareness. |
| `protocol` | Transport and route foundation: frames, route resolution, capability-aware authorization, json-line transport, and linked in-memory transport primitives. Independent leaf crate. |
| `bridge-runtime` | Shared managed bridge transport primitives for `http_json` and `process_stdio`, reused by both the spec rail and production bridge execution paths. |
| `app` | Product/runtime layer. Owns providers, channels, tools, memory backends, chat/conversation/session logic, config loading, runtime environment helpers, and presentation-facing surfaces. Houses the feature-flagged product modules. |
| `spec` | Deterministic execution rail. Owns runner specs, bootstrap builders, programmatic tool/spec execution, and test-facing runtime scaffolding that should stay out of daemon business logic. |
| `bench` | Performance and pressure rail. Owns benchmark suites and gate enforcement on top of the spec/kernel surfaces instead of folding that logic into the normal runtime path. |
| `daemon` | Operator assembly layer. `loong` is the supported command-line entrypoint. Wires lower-layer crates into CLI and service entrypoints such as `onboard`, `ask`, `chat`, `doctor`, `gateway`, `tasks`, `skills`, plugin flows, migration flows, and benchmarks. |

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
| Tool | Built-in tools (`shell.exec`, `file.read`, `file.write`, `file.edit`) | Community tool adapters |
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

Read the deeper design spec before changing semantics at these layers. This
file is the map, not the full per-layer contract.

## Design Principles

These are the core principles for anyone working in this codebase. They are enforced
mechanically where possible.

Some of these principles are durable invariants, while others describe the
current preferred architecture shape. The right way to change the latter is not
to quietly code around them, but to first make the design case and then update
the documented contract deliberately.

1. **Kernel-first** -- kernel-governed execution routes through the kernel's capability, policy, and audit system. Remaining direct compatibility paths must be explicit and are follow-up work, not implicit shadow routing.
2. **No breaking changes** -- new features are additive only. Existing public API signatures stay unchanged.
3. **Capability-gated by default** -- every tool call, memory operation, and connector invocation requires a valid `CapabilityToken`.
4. **Audit everything security-critical** -- policy denials, token lifecycle events, and module invocations all emit structured audit events.
5. **8-crate DAG, no cycles** -- dependency direction is non-negotiable.
6. **Tests first** -- if a behavior isn't tested, it doesn't exist. All tests pass at every commit.
7. **Proven technology preferred** -- choose well-understood, composable dependencies over opaque packages.
8. **Repository is the system of record** -- design decisions and architectural context live in `docs/`, not in chat threads.
9. **Automate first** -- prefer linters, CI gates, and pre-commit hooks over code review comments.
10. **Strictly avoid over-engineering** -- the minimum complexity for the current task is the right amount.

## Further Reading

| Topic | Document |
|-------|----------|
| Repository docs layering | [Repository Docs Map](docs/README.md) |
| Full layer specification (L0-L9) | [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) |
| Runtime/bootstrap surface map | [Runtime Entrypoint and Bootstrap Map](docs/design-docs/runtime-entrypoint-map.md) |
| Harness engineering & backpressure | [Harness Engineering](docs/design-docs/harness-engineering.md) |
| Design decisions, patterns & catalog | [Design Docs Index](docs/design-docs/index.md) |
| Security model & gaps | [Security](docs/SECURITY.md) |
| Stage-based roadmap | [Roadmap](docs/ROADMAP.md) |
| Build and kernel invariants | [Reliability](docs/RELIABILITY.md) |
| Product principles | [Product Sense](docs/PRODUCT_SENSE.md) |
| Repository support references | [References Index](docs/references/README.md) |
| Release support conventions | [Release Docs Convention](docs/releases/README.md) |
| Contributor workflow and recipes | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Examples and spec files | [Examples](examples/README.md) |
