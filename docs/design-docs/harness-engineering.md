# Harness Engineering in LoongClaw

> Based on [OpenAI's harness engineering framework](https://openai.com/index/harness-engineering/) (February 2026), mapped to LoongClaw's architecture.

## What is Harness Engineering?

Harness engineering is designing the full environment of scaffolding, constraints, and feedback loops surrounding AI agents. It sits above prompt engineering and context engineering in a three-layer hierarchy:

| Layer | Question | Design Target |
|-------|----------|---------------|
| Prompt Engineering | "What should be asked?" | Instruction text to the LLM |
| Context Engineering | "What should be shown?" | All tokens visible during reasoning |
| **Harness Engineering** | "How should the whole environment be designed?" | External constraints, feedback loops, operational systems |

**Central thesis**: The bottleneck in agent performance is often not model intelligence, but environment design.

---

## LoongClaw's Harness Components

### Stage 1: Intent Capture & Orchestration

| Component | Location | What it does |
|-----------|----------|--------------|
| `HarnessBroker` | `kernel/src/harness.rs` | Routes `TaskIntent` to registered `HarnessAdapter` by `ExecutionRoute` |
| `HarnessAdapter` trait | `kernel/src/harness.rs` | Async trait: `name()`, `kind()`, `execute(HarnessRequest) -> HarnessOutcome` |
| `HarnessKind` enum | `contracts/src/contracts.rs` | Two dispatch kinds: `EmbeddedPi` (in-process), `Acp` (external protocol) |
| `ExecutionRoute` | `contracts/src/contracts.rs` | Pack-level default route binding harness kind to adapter |
| `VerticalPackManifest` | `contracts/src/pack.rs` | Domain packaging: capabilities, allowed connectors, default route |

### Stage 2: Tool Call Execution

The intended capability gate for every tool call:

```
CapabilityToken → PolicyEngine → PolicyExtensionChain → ToolPlane → CoreToolAdapter → Audit
```

**Current reality**: Only `shell.exec` passes through the full PolicyEngine check. `file.read`, `file.write`, and `file.edit` have path sandboxing but bypass the policy engine entirely (TD-002). This means the Rule of Two (LLM intent + deterministic policy approval) is only enforced for shell commands.

Current tool registry: `shell.exec`, `file.read`, `file.write`, `file.edit`.

### Stage 3: Context Management & Memory

| Layer | Status |
|-------|--------|
| Working context (system prompt + tool snapshot + sliding window) | Implemented |
| Session state (SQLite turns table) | Implemented |
| Long-term memory | Not implemented |

### Stage 4: Result Verification & Iteration

- `ConversationTurnLoop`: Multi-round agent loop (max 4 rounds default)
- `ToolLoopSupervisor`: Detects infinite loops, ping-pong, failure streaks
- `FollowupPayloadBudget`: Caps tool output size per round

### Stage 5: Completion and Handoff

Turn persistence to SQLite. Audit event recording. Structured progress artifacts are a gap.

---

## Architectural Constraints as Harness

### Compile-Time Backpressure (Upstream)

The workspace clippy configuration mechanically prevents agent-generated anti-patterns:

| Lint | Why |
|------|-----|
| `unwrap_used`, `expect_used` | Forces proper error handling |
| `panic`, `todo`, `unimplemented` | Prevents incomplete stubs |
| `indexing_slicing` | Forces bounds-checked `.get()` |
| `print_stdout`, `print_stderr` | Prevents debug output leaking |
| `unsafe_code` | No unsafe in the workspace |

### Dependency DAG as Constraint

The 7-crate DAG prevents circular dependencies and implementation leakage. Enforced by `scripts/check_dep_graph.sh` and `task check:architecture`.

### Testing as Downstream Backpressure

8 test tiers (T0-T7) from [Layered Kernel Design](layered-kernel-design.md) provide downstream constraints, from contract serialization tests to self-governance architecture guards.

### Pre-Commit Hook as Gate

`scripts/pre-commit` runs CI-parity cargo checks before every commit.

---

## The Backpressure Principle

The ratio of upstream + downstream constraints determines maximum safe agent autonomy:

```
Upstream constraints              Downstream constraints
(compile-time lints,              (tests, CI gates,
 type system, DAG,                 pre-commit hooks,
 policy engine)                    audit trail)
        |                                  |
        +------------------+---------------+
                           |
                  Maximum safe autonomy
```

LoongClaw's position: **strong upstream** (strict lints, capability tokens, policy engine, type-safe contracts) + **strong downstream** (CI workflows, pre-commit hook, convention engineering, architecture checks).

---

## Context Files as System of Record

Progressive disclosure hierarchy:

| Tier | Files | Loading |
|------|-------|---------|
| Hot | `AGENTS.md` / `CLAUDE.md` | Auto-loaded every session |
| Specialized | Design docs, domain indices | Loaded when working on that domain |
| Cold | Roadmap, reliability, product specs, plans | Accessed on demand |

---

## References

- [OpenAI: Harness Engineering](https://openai.com/index/harness-engineering/) (February 2026)
- [Martin Fowler: Harness Engineering](https://martinfowler.com/articles/exploring-gen-ai/harness-engineering.html)
- [Layered Kernel Design](layered-kernel-design.md)
- [Core Beliefs](core-beliefs.md)
