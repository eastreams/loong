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

**Current reality**: Only `shell.exec` passes through the full PolicyEngine check. `file.read` and `file.write` have path sandboxing but bypass the policy engine entirely (TD-002). This means the Rule of Two (LLM intent + deterministic policy approval) is only enforced for shell commands.

Current tool registry: `shell.exec`, `file.read`, `file.write`.

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

## Key Gaps for Harness Maturity

Ranked by impact on agent reliability. All tracked in [Tech Debt Tracker](../TECH_DEBT.md).

### High Priority

1. **Rule of Two incomplete** (TD-002) — Policy engine only gates `shell.exec`. File and runtime tools bypass policy entirely.
2. **No process isolation** (TD-019) — Shell commands run without seccomp/Landlock/sandbox_init.
3. **Audit forgeable** (TD-020) — Signing key shares process with plugins.
4. **Persistent Audit Sink** (TD-006) — Audit events are in-memory only, lost on restart.
5. **No HMAC chain** (TD-007) — Audit events have no tamper evidence.

### Medium Priority

6. **Context Engine** (TD-008) — No pluggable context assembly. Conversation loop hardcodes sliding window.
7. **Observation Masking** (TD-021) — Old tool outputs not replaced with placeholders when context nears limits.
8. **Memory scopes absent** (TD-010) — Flat session_id, no Task/Session/Agent/Global scoping.
9. **MemoryStore trait** (TD-022) — String dispatch instead of typed methods.
10. **Provenance absent** (TD-014) — Memory entries lack the 10 mandatory fields from D-019.

### Lower Priority

11. **Token-Aware Budgets** (TD-009) — Payload budget tracks characters, not tokens.
12. **No FTS5 index** (TD-011) — Recency-only retrieval, no full-text search.
13. **No materialized views** (TD-018) — Event log exists but no derived state.

---

## References

- [OpenAI: Harness Engineering](https://openai.com/index/harness-engineering/) (February 2026)
- [Martin Fowler: Harness Engineering](https://martinfowler.com/articles/exploring-gen-ai/harness-engineering.html)
- [Layered Kernel Design](layered-kernel-design.md)
- [Core Beliefs](core-beliefs.md)
