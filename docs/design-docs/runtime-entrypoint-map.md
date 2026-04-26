# Runtime Entrypoint and Bootstrap Map

This document is the repository-native reading map for Loong's runtime
entrypoints.

It exists for contributors who can already find the code, but want to know
which bootstrap/helper surface to open first and what each one deliberately
owns or does **not** own.

## Read This Document When

- you are comparing `init` / `bootstrap` / `run_turn` variants that look
  similar but are not interchangeable
- you need to trace how a turn enters the shared runtime from CLI, channels,
  gateway, control plane, or daemon task execution
- you are deciding whether a new surface should mint fresh kernel/ACP state or
  reuse authority that an outer host already owns

## Shared Bootstrap Primitives

These helpers are the main seam lines. Reading them in this order usually gives
the fastest mental model.

| Helper | Location | Owns | Deliberately does **not** own |
| --- | --- | --- | --- |
| `bootstrap_kernel_context_with_config` | `crates/app/src/context.rs` | audit sink selection, MVP pack registration, tool/memory adapter registration, policy extensions, capability token issuance | process env export, session selection, channel/conversation state |
| `export_runtime_environment` | `crates/app/src/runtime_env.rs` | `LOONG_*` env export for child-process and legacy env-driven surfaces | kernel bootstrap, singleton runtime config init, session selection |
| `initialize_runtime_singletons` | `crates/app/src/runtime_env.rs` | in-process tool/memory runtime config initialization | process env export, kernel bootstrap, session selection |
| `initialize_runtime_environment` | `crates/app/src/runtime_env.rs` | compatibility wrapper that runs env export plus singleton init | kernel bootstrap, session selection, durable turn state |
| `initialize_cli_turn_runtime` | `crates/app/src/turn_runtime.rs` | config load, optional runtime env export, fresh kernel bootstrap, implicit default session allowance | channel-owned kernel reuse, ACP manager reuse |
| `initialize_cli_turn_runtime_with_loaded_config` | `crates/app/src/turn_runtime.rs` | runtime assembly from an already loaded config, fresh kernel bootstrap, singleton runtime-config init | config reload from disk, kernel/ACP reuse from an outer host |
| `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx` | `crates/app/src/turn_runtime.rs` | ACP defaults, memory/sqlite prep, session id/address derivation, task-scope derivation, `CliTurnRuntime` assembly | env export, fresh kernel bootstrap |
| `load_runtime_turn_config` | `crates/app/src/agent_runtime.rs` | provider-facing config refresh for long-lived hosts | channel account re-resolution, full runtime rebuild |
| `reload_channel_turn_config` | `crates/app/src/channel/dispatch.rs` | channel turn-time provider refresh | serve-loop account selection, serve runtime mutation |

## Surface Map

| Surface | Main entrypoint | Shared pieces it reuses | What makes it different |
| --- | --- | --- | --- |
| CLI chat / ask | `crates/app/src/chat.rs` → `run_cli_chat`, `run_cli_ask` | `turn_runtime` bootstrap, `AgentRuntime`, conversation runtime | owns the full user-facing runtime shell, can fall back to implicit/default session |
| Generic agent runtime | `crates/app/src/agent_runtime.rs` → `run_turn`, `run_turn_with_loaded_config`, `run_turn_with_loaded_config_and_acp_manager` | shared `turn_runtime` assembly + provider/ACP execution | transport-neutral wrapper used by multiple outer surfaces |
| Long-running channel serve | `crates/app/src/channel/commands/serve.rs` and `channel/runtime/serve.rs` | `initialize_runtime_environment`, `bootstrap_kernel_context_with_config` | owns serve-loop kernel authority and singleton runtime slot tracking |
| Channel inbound message bridge | `crates/app/src/channel/dispatch.rs` → `process_inbound_with_provider` | channel-owned `kernel_ctx`, `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx`, `AgentRuntime` | reuses an already bootstrapped kernel because the outer serve loop already owns it |
| Gateway HTTP turn | `crates/daemon/src/gateway/api_turn.rs` → `handle_turn` | loaded config snapshot, shared ACP manager, `AgentRuntime::run_turn_with_loaded_config_and_acp_manager` | always executes as an ACP turn; no interactive runtime shell |
| Control plane turn submit | `crates/daemon/src/control_plane_server.rs` → `/turn/submit` path + `crates/daemon/src/control_plane_turn_runtime.rs` | loaded config snapshot, shared ACP manager, per-turn registry, `AgentRuntime::run_turn_with_loaded_config_and_acp_manager` | turn execution only exists when the control plane was launched with a concrete config |
| Daemon task/turn CLI | `crates/daemon/src/task_execution.rs` → `run_turn_cli`, `execute_daemon_task_with_supervisor` | kernel task supervisor + embedded harness + `AgentRuntime` | deliberately routes turns through the same task/harness lane the daemon uses for generic task execution |
| Background tasks create | `crates/daemon/src/tasks_cli.rs` → `build_tasks_create_runtime` | detached sqlite runtime when available | prefers detached/background-safe runtime instead of foreground CLI lifetime |

## Call Paths by Surface

### 1. CLI chat / ask

```text
run_cli_chat / run_cli_ask
  -> initialize_cli_turn_runtime
  -> initialize_cli_turn_runtime_with_loaded_config
  -> export_runtime_environment (optional)
  -> initialize_runtime_singletons
  -> bootstrap_kernel_context_with_config
  -> initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx
  -> AgentRuntime / ConversationTurnCoordinator
```

Use this path as the baseline for answering “what does a full fresh turn
bootstrap look like?”

### 2. Generic `AgentRuntime` wrappers

`AgentRuntime` is intentionally a **transport-neutral veneer**:

- `run_turn` is the simplest path: load config, assemble turn runtime, run turn
- `run_turn_with_loaded_config` skips config load but still bootstraps a fresh
  turn runtime
- `run_turn_with_loaded_config_and_acp_manager` also reuses an existing ACP
  manager, which matters for gateway/control-plane style hosts that should share
  ACP session ownership across turns

### 3. Channel inbound bridge

```text
channel webhook/socket receive
  -> process_inbound_with_provider
  -> reload_channel_turn_config
  -> resolve_channel_acp_turn_hints
  -> initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx
  -> AgentRuntime::run_turn_with_runtime_...
```

Important distinction:

- the outer channel serve loop already owns the governed `kernel_ctx`
- the inbound bridge must **reuse** that authority instead of minting another
  kernel token for each message

### 4. Gateway HTTP turn

```text
gateway/api_turn.rs::handle_turn
  -> validate target address
  -> reuse app_state config + ACP manager
  -> AgentRuntime::run_turn_with_loaded_config_and_acp_manager
```

This path is narrower than CLI chat:

- it always executes as an ACP turn
- it does not own long-lived interactive runtime state
- it behaves more like “submit one governed turn to the ACP lane”

### 5. Control plane `/turn/submit`

```text
run_control_plane_serve_cli
  -> ControlPlaneTurnRuntime::new
  -> HTTP /turn/submit handler
  -> submit_control_plane_turn
  -> per-turn registry issue_turn(...)
  -> AgentRuntime::run_turn_with_loaded_config_and_acp_manager
```

Two subtleties matter here:

1. `ControlPlaneTurnRuntime` is intentionally a **narrow shell**, not the full
   router state
2. no-config control-plane mode can still expose control/read routes, but it
   cannot synthesize governed chat turns

### 6. Daemon task / turn path

```text
run_turn_cli
  -> build_daemon_runtime_kernel
  -> execute_daemon_task_with_supervisor
  -> EmbeddedAgentHarness::execute
  -> AgentRuntime::run_turn
```

This is the path to read when the question is not “how does chat work?” but
“how does a daemon task eventually land in the same runtime?”

## Reading Guide by Problem

| If you are debugging... | Start here | Then open |
| --- | --- | --- |
| why a CLI/chat turn picked the wrong session | `crates/app/src/turn_runtime.rs` | `chat.rs`, `agent_runtime.rs`, `session_*` helpers |
| why ACP is or is not used for a turn | `crates/app/src/agent_runtime.rs` | `channel/dispatch.rs`, `control_plane_server.rs`, `gateway/api_turn.rs` |
| why a channel message reused or did not reuse kernel authority | `crates/app/src/channel/dispatch.rs` | `turn_runtime.rs`, `context.rs` |
| why gateway/control-plane turns behave differently from chat | `gateway/api_turn.rs` or `control_plane_server.rs` | `agent_runtime.rs` |
| why a daemon task path differs from direct chat execution | `daemon/src/task_execution.rs` | `agent_runtime.rs`, `turn_runtime.rs`, `chat.rs` |
| why background task creation uses a detached runtime | `daemon/src/tasks_cli.rs` | `conversation` runtime implementations |

## Guardrails for Future Modifiers

When adding a new surface, decide these questions explicitly before writing the
code:

1. **Does this surface already own kernel authority?**
   - If yes, prefer reusing `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx`
   - If no, bootstrap fresh authority through the higher-level chat helpers

2. **Does this surface need to share ACP manager ownership across turns?**
   - If yes, reuse `AgentRuntime::run_turn_with_loaded_config_and_acp_manager`
   - If no, the simpler `run_turn` or `run_turn_with_loaded_config` is safer

3. **Is only provider-facing state supposed to refresh between turns?**
   - If yes, use `load_runtime_turn_config` / `reload_channel_turn_config`
   - Do not silently rebuild unrelated serve-loop state

4. **Is this surface interactive, one-shot, or daemon-supervised?**
   - The answer should show up in the chosen entrypoint, not be hidden inside a
     late-stage conditional

## Related Documents

- [Layered Kernel Design](layered-kernel-design.md)
- [Harness Engineering](harness-engineering.md)
- [Architecture Map](../../ARCHITECTURE.md)
