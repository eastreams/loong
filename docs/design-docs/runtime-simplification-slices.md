# Runtime / Tools / Config Simplification Slices

## Status

Active

## Summary

This document records the current repository-native review for the
"collapse loop/runtime/tools/config complexity" lane.

The goal is not to force Loong into a different product shape. The goal is to
keep the current single-entry product while making the remaining complexity more
explicit and easier to refactor in bounded slices.

The recommendations here are grounded in the current Loong codebase. The task
framing is pi-mono-aligned simplification, but the evidence below is taken from
Loong's existing source rather than from a direct comparator import.

## What Is Already In Good Shape

Several important seams are already converged and should stay intact:

- [`crates/app/src/agent_runtime.rs`](../../crates/app/src/agent_runtime.rs)
  already exposes `TurnExecutionService`, which is the shared host-facing seam
  for executing a turn from a loaded config snapshot.
- [`crates/daemon/src/gateway/api_turn.rs`](../../crates/daemon/src/gateway/api_turn.rs)
  is already a thin ACP host: it validates the request, builds an
  `AgentTurnRequest`, and delegates execution to `TurnExecutionService`.
- [`crates/daemon/src/control_plane_server.rs`](../../crates/daemon/src/control_plane_server.rs)
  uses the same execution seam for `/turn/submit`, including the shared ACP
  manager and the opt-out from redundant runtime-environment export.
- [`docs/design-docs/tool-surface-exposure.md`](tool-surface-exposure.md)
  already states the intended direct-tool versus hidden-tool split, including
  the `tool.search -> tool.invoke` lease flow. Simplification work should keep
  that contract stable.

In other words: the main problem is no longer "every host has a different turn
engine." The main problem is that some oversized files still mix runtime
assembly, loop ownership, tool/config projection, and host shell code.

## Current Hotspot Inventory

| Area | Current evidence | Why it is still a hotspot |
| --- | --- | --- |
| CLI loop + runtime assembly | [`crates/app/src/chat.rs`](../../crates/app/src/chat.rs) is about 6k lines and still contains missing-config onboarding, CLI loop control, render helpers, plus `initialize_cli_turn_runtime*` assembly helpers | CLI shell concerns and reusable runtime assembly still live in one file, which makes follow-up refactors feel riskier than they need to be |
| Shared turn host seam | [`crates/app/src/agent_runtime.rs`](../../crates/app/src/agent_runtime.rs) is about 1k lines and contains `TurnExecutionService`, `RuntimeTurnExecutionService`, config refresh, and prompt-summary reporting | This seam is good, but it still depends on chat-owned runtime assembly helpers, so its boundaries are only partially visible |
| Runtime environment projection | [`crates/app/src/runtime_env.rs`](../../crates/app/src/runtime_env.rs) exports `LOONG_*` variables and also initializes singleton tool/memory runtime caches from the same helper | Process-env compatibility and in-process config projection are coupled even though callers may need only one of those behaviors |
| Control-plane host shell | [`crates/daemon/src/control_plane_server.rs`](../../crates/daemon/src/control_plane_server.rs) is about 5k lines and contains router setup, auth, runtime shell structs, per-turn event forwarding, and `/turn/submit` execution | The control-plane turn path is converged logically, but the host file still hides that good shape inside a very large server module |

## Highest-Leverage Next Slices

These are the next slices that best reduce complexity without changing product
behavior or the 7-crate DAG.

### Slice 1: move reusable runtime assembly out of `chat.rs`

Target:

- `initialize_cli_turn_runtime`
- `initialize_cli_turn_runtime_with_loaded_config`
- `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx`
- session-requirement helpers that only exist to support those entrypoints

Why this is the highest-leverage slice:

- `TurnExecutionService` already gives gateway and control-plane hosts a shared
  execution seam.
- The remaining reuse bottleneck is that those hosts still depend on runtime
  assembly helpers that sit beside CLI REPL and onboarding logic in
  `chat.rs`.
- Extracting a small runtime-assembly module would make the existing
  convergence visible without changing behavior.

Non-goals:

- do not move CLI rendering, REPL loop control, or missing-config onboarding
  into a generic runtime module
- do not create a new crate only for this extraction

### Slice 2: narrow the control-plane turn shell

Target:

- `ControlPlaneTurnRuntime`
- `ControlPlaneTurnEventForwarder`
- the `/turn/submit` execution helper path

Why this matters:

- the runtime path itself is already good because it delegates through
  `TurnExecutionService`
- the remaining cost is discoverability: turn execution is buried inside a
  large server/router file

Recommended shape:

- move the turn-runtime shell into a dedicated control-plane submodule or
  include file
- keep the top-level server file focused on routing, authorization, and shared
  control-plane lifecycle

### Slice 3: split runtime-env export from runtime-config initialization

Today, `initialize_runtime_environment(...)` does two different jobs:

1. export `LOONG_*` process variables for child-process and legacy
   environment-driven surfaces
2. initialize in-process tool and memory runtime singletons

Why this matters:

- some hosts intentionally call `TurnExecutionService::without_runtime_environment_init()`
  because they do not want to re-export environment state on every turn
- that is a signal that "runtime environment" is still carrying more than one
  concern

Recommended shape:

- keep the current public behavior stable
- internally separate env export from singleton runtime-config projection so
  hosts can opt into the exact side effects they need

### Slice 4: keep tool-surface simplification documentation-first

The tool plane should stay aligned with
[`tool-surface-exposure.md`](tool-surface-exposure.md):

- direct tools stay directly visible
- hidden specialized tools stay behind `tool.search`
- invocation stays lease-bound through `tool.invoke`

That means tool-complexity cleanup should prefer:

- extracting helper modules near the existing catalog / prompt / discovery code
- reducing file-size pressure without changing the progressive-disclosure
  contract

It should avoid:

- collapsing hidden-tool discovery into ad-hoc direct-tool exposure
- re-coupling runtime bootstrap work with tool-surface policy decisions

## Review Guidance For Implementers

When working this lane, prefer the following order:

1. extract reusable runtime assembly from `chat.rs`
2. narrow the control-plane host shell
3. split runtime-env side effects
4. only then consider deeper file-size cleanup in tool surfaces

That order matters because it preserves the already-correct shared execution
seam before touching policy-sensitive tool exposure behavior.

## Related Documents

- [Runtime Entrypoint and Bootstrap Map](runtime-entrypoint-map.md)
- [Single-Entry Runtime Convergence](single-entry-runtime-convergence.md)
- [Tool Surface Exposure](tool-surface-exposure.md)
- [Core Beliefs](core-beliefs.md)
