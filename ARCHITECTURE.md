# Architecture

LoongClaw is an agent execution kernel with an MVP chat/channel layer. The workspace is split into 7 crates with a strict dependency DAG — no cycles allowed.

## Crate Dependency Diagram

```
contracts (leaf — zero internal deps)
├── kernel → contracts
├── protocol (independent leaf)
├── app → contracts, kernel
├── spec → contracts, kernel, protocol
├── bench → contracts, kernel, spec
└── daemon (binary) → all of the above
```

## Crate Responsibilities

| Crate | Lines | Purpose |
|-------|-------|---------|
| `contracts` | ~566 | Shared types, traits, errors. Leaf crate with zero workspace deps. |
| `kernel` | ~5,700 | Policy engine, 4 execution planes (Connector, Runtime, Tool, Memory), capability tokens, audit trail. |
| `protocol` | ~833 | Transport traits, JSON-line framing, execution routing. |
| `app` | ~5,600 | Config, LLM providers, channels (CLI/Telegram/Feishu), conversation orchestration, SQLite memory, shell/file tools. |
| `spec` | ~8,000 | Spec runner, execution engine, KernelBuilder, programmatic tool calls. |
| `bench` | ~2,300 | Pressure benchmarks, programmatic stress testing. |
| `daemon` | ~500 | CLI binary (clap), subcommand dispatch. No business logic. |

## Data Flow: One Chat Message

```
User input
  → run_cli_chat (app/chat.rs)
    → bootstrap_kernel_context("cli-chat", 86400)
      → LoongClawKernel + CapabilityToken
    → ConversationOrchestrator::handle_turn(config, session, input, Some(&ctx))
      → runtime.build_messages(config, session, true, kernel_ctx)
        → provider::build_messages_for_session  (loads system prompt + memory window)
      → runtime.request_completion(config, messages)
        → HTTP POST to LLM provider (OpenAI-compatible)
      → runtime.persist_turn(session, "user", input, kernel_ctx)
        → kernel.execute_memory_core(pack_id, token, {MemoryWrite}, request)
          → MvpMemoryAdapter → SQLite
      → runtime.persist_turn(session, "assistant", reply, kernel_ctx)
        → same kernel-routed path
  → print reply
```

When `KernelContext` is present, memory and tool operations route through the kernel's capability/policy/audit system. When absent (e.g., tests), they fall back to direct adapter calls.

## Where Does X Live?

| Thing | Location |
|-------|----------|
| Capability enum | `crates/contracts/src/contracts.rs` |
| CapabilityToken | `crates/contracts/src/contracts.rs` |
| Error types | `crates/contracts/src/errors.rs` |
| Clock trait | `crates/contracts/src/clock.rs` |
| Fault enum | `crates/contracts/src/fault.rs` |
| TaskState FSM | `crates/contracts/src/task_state.rs` |
| Namespace | `crates/contracts/src/namespace.rs` |
| AuditEvent types | `crates/contracts/src/audit_types.rs` |
| Policy types | `crates/contracts/src/policy_types.rs` |
| LoongClawKernel | `crates/kernel/src/kernel.rs` |
| PolicyEngine trait | `crates/kernel/src/policy.rs` |
| CoreToolAdapter trait | `crates/kernel/src/tool.rs` |
| CoreMemoryAdapter trait | `crates/kernel/src/memory.rs` |
| TaskSupervisor | `crates/kernel/src/task_supervisor.rs` |
| KernelBuilder | `crates/spec/src/kernel_bootstrap.rs` |
| KernelContext | `crates/app/src/context.rs` |
| ConversationRuntime | `crates/app/src/conversation/runtime.rs` |
| ConversationOrchestrator | `crates/app/src/conversation/orchestrator.rs` |
| LLM provider config | `crates/app/src/provider/mod.rs` |
| Shell/file tools | `crates/app/src/tools/` |
| SQLite memory | `crates/app/src/memory/sqlite.rs` |
| Channel adapters | `crates/app/src/channel/` |
| CLI chat loop | `crates/app/src/chat.rs` |
| Config loading | `crates/app/src/config/` |
| Spec runner | `crates/spec/src/spec_execution/` |
| Pressure benchmarks | `crates/bench/src/lib.rs` |
| CLI entry point | `crates/daemon/src/main.rs` |

## Kernel Execution Model

The kernel enforces a **capability-gated, audited execution pipeline**:

1. Entry point bootstraps `KernelContext` with a `CapabilityToken` scoped to a pack
2. Operations check token capabilities (e.g., `InvokeTool`, `MemoryWrite`)
3. The policy engine authorizes or denies each operation
4. Audit events are recorded for every plane invocation and denial
5. The registered adapter (e.g., `MvpToolAdapter`) executes the actual work

## Feature Flags

The `app` crate uses feature flags for optional functionality:

| Flag | Controls |
|------|----------|
| `memory-sqlite` | SQLite-backed conversation memory |
| `channel-telegram` | Telegram bot channel |
| `channel-feishu` | Feishu/Lark channel |
| `provider-openai` | OpenAI-compatible provider |
| `tools-shell` | Shell command execution |
| `tools-file` | File read/write tools |

Build with no optional features: `cargo build -p loongclaw-app --no-default-features`

## Kernel Primitives (Phase 3)

These were added as purely additive changes — no breaking modifications to existing APIs:

- **Generation + Membrane on tokens** — `CapabilityToken.generation` enables O(1) bulk revocation via `revoke_generation(below)`. `membrane: Option<String>` carries a namespace isolation tag for future use.
- **Fault enum** — structured error type for runtime dispatch failures with variants: `Panic`, `CapabilityViolation`, `BudgetExhausted`, `TokenExpired`, `ProtocolViolation`, `PolicyDenied`. Includes conversion from `PolicyError` and `KernelError`.
- **TaskState FSM** — state machine for task lifecycle: `Runnable → InSend → InReply → Completed | Faulted`. `TaskSupervisor` wraps `execute_task` with FSM enforcement (opt-in).
- **Namespace** — runtime projection of `VerticalPackManifest`, created during `register_pack`. Membrane field defaults to `pack_id` for isolation tagging.
