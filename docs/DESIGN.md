# Design

High-level index for design philosophy, decisions, and patterns in LoongClaw.

## Philosophy

LoongClaw follows an **agent-first** design philosophy where the repository itself is the system of record. Design decisions, architectural context, and engineering principles live in code and docs, not in chat threads. See [Core Beliefs](design-docs/core-beliefs.md) for the full set of golden principles.

## Architecture

The kernel enforces a layered execution model (L0-L9) with strict dependency direction. Every execution path routes through capability, policy, and audit gates. See [ARCHITECTURE.md](../ARCHITECTURE.md) for the crate DAG and layer overview.

Full specification: [Layered Kernel Design](design-docs/layered-kernel-design.md)

## Design Documents

See [Design Docs Index](design-docs/index.md) for the full catalog of design documents and tracked decisions.

## Key Patterns

| Pattern | Description | Where Enforced |
|---------|-------------|----------------|
| Core/Extension split | Every execution plane has a core adapter and optional extensions | `kernel/src/tool.rs`, `runtime.rs`, `memory.rs`, `connector.rs` |
| Capability-gated access | Every resource access requires an explicit capability token | `kernel/src/policy.rs` |
| Rule of Two | Tool calls require both LLM intent and deterministic policy approval | `kernel/src/policy.rs` |
| Registry pattern | Adapters registered by name into `BTreeMap<String, Arc<dyn Trait>>` | All execution planes |
| Generation-based revocation | `AtomicU64` threshold invalidates all tokens with generation <= N | `kernel/src/kernel.rs` |
| Policy extension chain | Chain-of-responsibility: multiple extensions evaluated in order, any can deny | `kernel/src/policy_ext.rs` |

## Product Specifications

See [Product Specs Index](product-specs/index.md) for user-facing requirements.

## Plans

Active execution plans live in [`plans/`](plans/). See [Tech Debt Tracker](plans/tech-debt-tracker.md) for known architectural drift.
