# Harness Engineering in Loong

> Based on [OpenAI's harness engineering framework](https://openai.com/index/harness-engineering/) (February 2026), mapped to Loong's architecture.

## What is Harness Engineering?

Harness engineering is designing the full environment of scaffolding, constraints, and feedback loops surrounding AI agents. It sits above prompt engineering and context engineering in a three-layer hierarchy:

| Layer | Question | Design Target |
|-------|----------|---------------|
| Prompt Engineering | "What should be asked?" | Instruction text to the LLM |
| Context Engineering | "What should be shown?" | All tokens visible during reasoning |
| **Harness Engineering** | "How should the whole environment be designed?" | External constraints, feedback loops, operational systems |

**Central thesis**: The bottleneck in agent performance is often not model intelligence, but environment design.

---

## Loong's Harness Components

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

**Current reality**: Kernel-bound core tools still follow this gate. Tool-specific request approval now lives in policy extensions instead of the deprecated `PolicyEngine::check_tool_call` hook, so `shell.exec` is enforced by `ToolPolicyExtension` while `file.read`, `file.write`, and `file.edit` are enforced by `FilePolicyExtension` plus execution-layer path sandboxing. The Rule of Two is therefore still enforced on the kernel tool path, even though the request-policy split between engine and extensions remains architectural debt.

Current built-in core tool slice: `shell.exec`, `file.read`, `file.write`, `file.edit`.

### Stage 3: Context Management & Memory

| Layer | Status |
|-------|--------|
| Working context (system prompt + tool snapshot + sliding window) | Implemented |
| Session state (SQLite turns table) | Implemented |
| Long-term memory | Not implemented |

### Stage 4: Result Verification & Iteration

- `ConversationTurnCoordinator`: Active turn-bearing runtime seam across CLI,
  gateway, control plane, channel bridge, and daemon task paths
- `Execution Discipline` prompt fragment: Product-owned runtime guidance for
  tool persistence, act-don't-ask behavior, prerequisite checks, and
  verification before stopping
- `ToolLoopSupervisor` / provider-lane guards: Detect repeated-tool stalls,
  ping-pong, and failure streaks
- `FollowupPayloadBudget`: Caps tool output size per round

### Stage 5: Completion and Handoff

Turn persistence to SQLite and audit event recording remain in place. Runtime
self continuity and turn checkpoint artifacts are implemented, and durable
task-progress events now provide a session-visible MVP for long-running
ownership and completion state.

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

The 7-crate DAG prevents circular dependencies and implementation leakage.
Enforced by `scripts/check_dep_graph.sh` and, when the optional `task` CLI is
installed, `task check:architecture`.

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

Loong's position: **strong upstream** (strict lints, capability tokens, policy engine, type-safe contracts) + **strong downstream** (CI workflows, pre-commit hook, convention engineering, architecture checks).

---

## Context Files as System of Record

Progressive disclosure hierarchy:

| Tier | Files | Loading |
|------|-------|---------|
| Hot | `AGENTS.md` / `CLAUDE.md` | Auto-loaded every session |
| Specialized | Design docs, domain indices | Loaded when working on that domain |
| Cold | Roadmap, reliability, and knowledge-base specs/plans | Accessed on demand |

---

## References

- [Runtime Entrypoint and Bootstrap Map](runtime-entrypoint-map.md)
- [OpenAI: Harness Engineering](https://openai.com/index/harness-engineering/) (February 2026)
- [Martin Fowler: Harness Engineering](https://martinfowler.com/articles/exploring-gen-ai/harness-engineering.html)
- [Layered Kernel Design](layered-kernel-design.md)
- [Core Beliefs](core-beliefs.md)
