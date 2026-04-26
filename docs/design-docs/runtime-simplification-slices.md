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

## Architectural Stance

Loong should not grow separate "coding agent" and "non-coding agent" runtime
bases.

The coding-agent runtime is the base substrate.

Other agent experiences should be treated as modular capability layers on top
of that base:

- the turn loop stays one coding-grade execution core
- tool usage, recovery, and verification stay one governed execution core
- non-coding surfaces add domain modules, channel adapters, or product
  workflows without forking the base runtime model

That means simplification work should prefer:

- making the coding-grade core thinner, clearer, and more reliable
- keeping extension points modular above that core
- avoiding a second runtime path just because a product surface is less
  code-centric

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
engine." The remaining problem is that some oversized files still mix loop
ownership, tool/config projection, and host shell code even after the shared
turn-runtime seam is made explicit.

The same pattern now exists inside conversation turn coordination: the
provider-turn helpers have been split into adjacent modules, but the top-level
coordinator still owns lane-planning/state carriers and most seam-local tests.

## Current Hotspot Inventory

| Area | Current evidence | Why it is still a hotspot |
| --- | --- | --- |
| CLI loop shell | [`crates/app/src/chat.rs`](../../crates/app/src/chat.rs) is about 6k lines and still contains missing-config onboarding, CLI loop control, and render helpers | The reusable runtime assembly already moved out, but the user-facing CLI shell is still a very large surface that mixes multiple operator concerns |
| Shared turn host seam | [`crates/app/src/agent_runtime.rs`](../../crates/app/src/agent_runtime.rs) is about 1k lines and contains `TurnExecutionService`, `RuntimeTurnExecutionService`, config refresh, and prompt-summary reporting | This seam is much clearer now that it points at [`crates/app/src/turn_runtime.rs`](../../crates/app/src/turn_runtime.rs), but it still carries transport shaping and prompt-summary reporting in one file |
| Runtime environment projection | [`crates/app/src/runtime_env.rs`](../../crates/app/src/runtime_env.rs) now exposes separate env-export and singleton-init helpers plus the compatibility wrapper | The internal split is in place, but many hosts still call only the compatibility wrapper, so the narrower side-effect contract is not yet visible at every call site |
| Control-plane host shell | [`crates/daemon/src/control_plane_server.rs`](../../crates/daemon/src/control_plane_server.rs) is still about 5k lines, while the extracted turn shell now lives in [`crates/daemon/src/control_plane_turn_runtime.rs`](../../crates/daemon/src/control_plane_turn_runtime.rs) | The turn runtime shell is clearer now, but the top-level server file still owns a lot of route glue and result/stream handling in one place |
| Provider-turn coordinator shell | [`crates/app/src/conversation/turn_coordinator.rs`](../../crates/app/src/conversation/turn_coordinator.rs) is still about 8.6k lines even after extracting [`turn_coordinator_support.rs`](../../crates/app/src/conversation/turn_coordinator_support.rs), [`provider_turn_runtime.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_runtime.rs), [`provider_turn_reply.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_reply.rs), [`provider_turn_lane.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_lane.rs), and [`provider_turn_apply.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_apply.rs) | The provider-turn seam is clearer now, but lane-planning/state carriers and most regression coverage still live in the monolith instead of next to the extracted helpers |

## Highest-Leverage Next Slices

These are the next slices that best reduce complexity without changing product
behavior or the 7-crate DAG.

### Slice 1: move reusable runtime assembly out of `chat.rs` (landed)

Target:

- `initialize_cli_turn_runtime`
- `initialize_cli_turn_runtime_with_loaded_config`
- `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx`
- session-requirement helpers that only exist to support those entrypoints

What landed:

- the extracted helpers now live in
  [`crates/app/src/turn_runtime.rs`](../../crates/app/src/turn_runtime.rs)
- `agent_runtime.rs` depends on that runtime seam directly instead of importing
  runtime assembly from `chat.rs`
- `chat.rs` keeps the CLI shell and reuses the new seam instead of owning it

Non-goals:

- do not move CLI rendering, REPL loop control, or missing-config onboarding
  into a generic runtime module
- do not create a new crate only for this extraction

### Slice 2: narrow the control-plane turn shell (partially landed)

Target:

- `ControlPlaneTurnRuntime`
- `ControlPlaneTurnEventForwarder`
- the `/turn/submit` execution helper path

What landed:

- [`crates/daemon/src/control_plane_turn_runtime.rs`](../../crates/daemon/src/control_plane_turn_runtime.rs)
  now owns `ControlPlaneTurnRuntime`, the event forwarder, and the spawned turn
  execution helper
- `control_plane_server.rs` keeps the HTTP handler, request validation, and
  response mapping while delegating the runtime shell work outward

Recommended shape:

- keep moving route-specific turn helpers out of the top-level server file
- keep the top-level server file focused on routing, authorization, and shared
  control-plane lifecycle

### Slice 3: follow through on provider-turn coordinator extraction (partially landed)

Target:

- `ProviderTurnSessionState` / `ProviderTurnPreparation`
- provider-turn runtime / reply / lane / apply helpers
- the remaining provider-turn lane-plan, lane-execution, and loop-state carriers
- seam-local regression coverage for the extracted files

What landed:

- the provider-turn helper surface now lives across
  [`crates/app/src/conversation/turn_coordinator_support.rs`](../../crates/app/src/conversation/turn_coordinator_support.rs),
  [`provider_turn_runtime.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_runtime.rs),
  [`provider_turn_reply.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_reply.rs),
  [`provider_turn_lane.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_lane.rs),
  and [`provider_turn_apply.rs`](../../crates/app/src/conversation/turn_coordinator/provider_turn_apply.rs)
- `turn_coordinator.rs` now imports those helpers instead of keeping the whole
  provider-turn path inline

Recommended shape:

- keep moving provider-turn-only state carriers next to the extracted helper
  files instead of leaving them in the top-level coordinator shell
- keep moving seam-local regression tests beside the extracted files so the
  module boundaries and their verification surface line up
- leave the top-level `turn_coordinator.rs` focused on orchestration and
  cross-lane coordination rather than helper internals

### Slice 4: split runtime-env export from runtime-config initialization (landed internally)

Today, `initialize_runtime_environment(...)` does two different jobs:

1. export `LOONG_*` process variables for child-process and legacy
   environment-driven surfaces
2. initialize in-process tool and memory runtime singletons

Why this matters:

- some hosts intentionally call `TurnExecutionService::without_runtime_environment_init()`
  because they do not want to re-export environment state on every turn
- that is a signal that "runtime environment" is still carrying more than one
  concern

What landed:

- [`crates/app/src/runtime_env.rs`](../../crates/app/src/runtime_env.rs) now
  exposes `export_runtime_environment(...)` and
  `initialize_runtime_singletons(...)`
- `initialize_runtime_environment(...)` remains as the compatibility wrapper
- the extracted turn-runtime seam now skips env export when requested while
  still initializing in-process runtime singletons

### Slice 5: keep tool-surface simplification documentation-first

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

1. narrow the control-plane host shell
2. finish the provider-turn coordinator seam so state carriers and tests move
   with the extracted helper files
3. make more callers use the narrower runtime-env side-effect helpers directly
4. only then consider deeper file-size cleanup in tool surfaces

That order matters because the shared execution seam is now explicit, and the
next wins come from shrinking the remaining host shells plus finishing the
already-started coordinator seam work without touching policy-sensitive tool
exposure behavior.

## Related Documents

- [Runtime Entrypoint and Bootstrap Map](runtime-entrypoint-map.md)
- [Single-Entry Runtime Convergence](single-entry-runtime-convergence.md)
- [Tool Surface Exposure](tool-surface-exposure.md)
- [Core Beliefs](core-beliefs.md)
