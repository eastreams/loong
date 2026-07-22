# Runtime Entrypoint and Ownership Map

This document is the repository-native reading map for Loong's live runtime
entrypoints. It distinguishes long-lived owners from the borrowed execution
scope so new surfaces do not recreate the retired root-context model.

## Ownership Spine

```text
host
  -> Arc<Runtime<RuntimeContextFactory>>
       owns Kernel + typed ToolPlane
  -> Session
       owns stable identity, authority ceiling, config, mailbox, and backends
  -> Context<'a>
       borrows Runtime + Session for recursive execution
       -> ctx.tool(path)?.invoke(payload)
       -> ctx.access().fs()...

typed registry miss
  -> explicit legacy dispatcher
       owns bearer token and ToolCore envelope
```

`Context<'a>` is never a startup owner. A host retains Runtime and Session,
rematerializes the Session when durable policy may have changed, and constructs
Context inside the structured call that consumes it. Detached work moves owned
Runtime/Session state into its future and creates a fresh borrow there.

## Primary Construction Boundaries

| Boundary | Location | Owns | Does not own |
| --- | --- | --- | --- |
| `bootstrap_runtime_with_config` | `crates/app/src/runtime.rs` | configured Kernel, policy pipeline, audit sink, typed ToolPlane; legacy pack registration while fallback exists | Session identity, Context, turn state |
| `Session::from_config` | `crates/app/src/context/session/materialize.rs` | one owned Session materialized from runtime availability, config, and one canonical repository lineage snapshot | Runtime, Context, legacy bearer evidence |
| `Context::new` | `crates/app/src/context.rs` | borrowed recursive execution scope and effective capability view | independent lifecycle, token/pack, audit sink |
| `initialize_cli_turn_runtime*` | `crates/app/src/chat/boot.rs` | session selection plus assembly of Runtime, Session, coordinator, and explicit legacy fallback owner | a retained Context |
| `TurnExecutionService::with_runtime` | `crates/app/src/agent_runtime.rs` | reuse of an outer host's Runtime while materializing the requested Session | replacement Kernel or second ToolPlane |
| `DefaultLegacyToolDispatcher::with_config` | `crates/app/src/conversation/turn_engine_dispatcher.rs` | bearer evidence for unmatched or unmigrated ToolCore requests | typed Tool/Access authorization |

The `RuntimeContextFactory` marker has only the ContextFactory GAT. It does not
construct Context and must not become a service locator.

## Shared Turn Flow

For a non-ACP provider turn, the live path is:

```text
host-owned Runtime + Session
  -> load turn config
  -> Session::rematerialize(Runtime, config)
  -> Context::new(Runtime, rematerialized Session)
  -> ConversationTurnCoordinator
  -> TurnEngine prepares each tool intent
       -> registered path: Context::tool(path)?.invoke(payload)
       -> missing path: explicit LegacyToolDispatcher fallback
```

Typed registration wins once lookup succeeds. Policy denial, input failure,
execution failure, or audit failure after a typed hit must return from the typed
path; none of them may be reinterpreted as a reason to try legacy dispatch.

## Surface Map

| Surface | Main entrypoint | Runtime/Session ownership |
| --- | --- | --- |
| CLI chat / ask | `crates/app/src/chat.rs` | `initialize_cli_turn_runtime` builds one Runtime and selected Session for the interactive host |
| Generic agent API | `crates/app/src/agent_runtime.rs` | `TurnExecutionService` either bootstraps Runtime or reuses one supplied by an outer host, then materializes Session |
| Channel serve | `crates/app/src/channel/commands/serve.rs` and `channel/dispatch.rs` | serve loop owns one Runtime; each routed session is materialized against that Runtime |
| Gateway turn | `crates/daemon/src/gateway/api_turn.rs` | request service supplies loaded config and shared ACP state; structured execution owns the selected Session |
| Control-plane turn | `crates/daemon/src/control_plane_server/{support,turn}.rs` | `ControlPlaneTurnRuntime` owns shared Runtime/ACP manager; spawned turn moves the Runtime Arc and materializes Session inside the task |
| Daemon task execution | `crates/daemon/src/task_execution.rs` | supervised request passes the daemon Runtime into `TurnExecutionService::with_runtime` |
| Background task / delegate | `crates/app/src/conversation/runtime_delegate.rs` and daemon task surfaces | child Session derives from live parent authority; detached execution owns the child Session and Runtime Arc |

## Entrypoint Call Paths

### CLI chat / ask

```text
run_cli_chat / run_cli_ask
  -> initialize_cli_turn_runtime
  -> initialize_cli_turn_runtime_with_loaded_config
  -> bootstrap_runtime_with_config
  -> Session::from_config
  -> CliTurnRuntime { runtime, session, legacy_tools, ... }
  -> runtime.context() or per-turn Session::rematerialize + Context::new
```

`CliTurnRuntime::context()` is a short borrow for non-turn operations. Provider
and tool turns rematerialize first so a durable session-policy update cannot be
skipped by retaining an older Context.

### Channel ingress

```text
channel serve bootstrap
  -> bootstrap_runtime_with_config
  -> receive message
  -> initialize_cli_turn_runtime_with_loaded_config_and_runtime
  -> Session::from_config using the serve Runtime
  -> AgentRuntime / ConversationTurnCoordinator
```

The serve loop reuses Runtime, not Context. Minting a new Kernel or bearer token
for every typed message would split policy/audit/tool-plane authority.

### Gateway and control plane

Gateway and control-plane hosts reuse their configured Runtime and shared ACP
manager. A submitted turn moves cloned long-lived handles into the spawned
future, materializes its Session there, and creates Context only while running
the recursive execution. No `&Context` escapes into the `'static` task.

### Child and detached execution

- Inline recursive work derives a child Context whose capabilities are a subset
  of its parent.
- A subagent is an owned child Session. Its durable parent id is lookup evidence;
  executable authority is derived from the live parent Session or the canonical
  persisted lineage materializer.
- Detached work is registered with a runtime owner and moves an owned Session;
  transferring a Context reference is invalid because Context has no lifecycle
  independent of those owners.

## Guardrails

Before adding a runtime surface, answer these questions in code comments at the
ownership boundary:

1. Which component owns the Runtime, and is it reused rather than rebuilt?
2. Which component owns the Session and cancellation/lifecycle state?
3. Where is Session rematerialized before durable policy is observed?
4. In which structured call is `Context<'a>` borrowed?
5. Is legacy fallback entered only after a typed registry miss?

The following are architecture regressions:

- retaining an owned/root Context beside Runtime or Session
- placing pack/token, audit sink, or ToolCore envelopes in Context
- rebuilding a ToolPath from a provider display name
- treating typed denial or execution failure as legacy fallback eligibility
- constructing a child Session from host config when a live parent Session is available

## Related Documents

- [Core Beliefs](core-beliefs.md)
- [Layered Kernel Design](layered-kernel-design.md)
- [Reliability](../RELIABILITY.md)
- [Architecture Map](../../ARCHITECTURE.md)
