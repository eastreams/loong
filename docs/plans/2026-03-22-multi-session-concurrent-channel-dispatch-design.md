# Multi-Session Concurrent Channel Dispatch Design

**Issue:** `#301`  
**Branch target:** `upstream/dev`  
**Related:** `#293`, `#217`, `#218`

## Scope

Issue `#301` should add one in-process runtime owner that can run the MVP
surfaces concurrently in a single LoongClaw process:

- CLI
- Telegram
- Feishu / Lark

This slice includes:

- concurrent task ownership in one process
- per-surface task lifecycle tracking
- coordinated graceful shutdown
- health data usable by logs and tests

This slice does **not** include:

- a new top-level operator-facing `status` command
- restart / backoff / self-healing policy
- broader daemon / service / gateway ownership work from `#217`

## Problem Statement

LoongClaw already has real channel runtime entrypoints and per-channel runtime
state, but they still operate as separate long-lived command paths.

Current code already provides:

- channel serve entrypoints such as `run_telegram_channel(...)` and
  `run_feishu_channel(...)` in `crates/app/src/channel/mod.rs`
- daemon-level wrappers that execute each serve flow individually from
  `crates/daemon/src/main.rs` and `crates/daemon/src/lib.rs`
- persisted per-channel runtime heartbeats in
  `crates/app/src/channel/runtime_state.rs`

That means the missing capability for `#301` is not "create a new channel
runtime surface." The missing capability is process-level orchestration:

- spawn multiple runtime surfaces together
- observe their lifecycle coherently
- shut them down together
- fail coherently when one critical child fails

## Design Goal

Add a daemon-owned in-process runtime owner that composes existing channel and
CLI entrypoints without moving channel-specific logic into a new subsystem.

The runtime owner should be the smallest slice that makes concurrent MVP
operation real while preserving the existing boundary:

- `crates/app` owns channel execution
- `crates/daemon` owns process orchestration

## Chosen Design

Add a new daemon-owned runtime owner entrypoint and module that:

1. resolves which runtime surfaces should run
2. constructs a typed supervisor spec
3. keeps the interactive CLI in the foreground for the first slice
4. spawns one Tokio task per background channel surface
5. tracks each background task's lifecycle in shared in-memory state
6. coordinates graceful shutdown across all children
7. tears the whole runtime owner down if any required child fails unexpectedly

The existing channel runtime files remain the persisted truth for per-channel
serve instances. The new runtime owner adds a process-local view above that
state; it does not replace channel runtime persistence.

This first slice intentionally does **not** require the current blocking CLI
REPL to become a homogeneous background child task. The existing CLI chat path
is stdin-driven and blocking today, so the initial concurrency model should be:

- foreground interactive CLI host
- background supervised Telegram task
- background supervised Feishu task

That is enough to satisfy "CLI + Telegram + Feishu in one process" without
inventing a fake symmetry that the current code does not support.

To make that model operationally correct, the first slice must also add a small
foreground CLI shutdown seam. The current REPL blocks on stdin reads, so the
runtime owner needs an explicit way to interrupt or unwind the foreground host
when:

- root shutdown begins
- a required background channel child fails

The design should therefore extract a cancellable foreground host loop rather
than keeping raw blocking stdin reads embedded in the top-level runtime owner.

## Component Boundaries

### `crates/daemon/src/supervisor.rs`

Add a new daemon-owned module responsible for:

- supervisor spec construction
- child task spawn orchestration
- in-memory lifecycle state map
- shutdown fan-out
- join handling and final exit semantics

Suggested core types:

- `SupervisorSpec`
- `SupervisedSurface`
- `SurfaceHandle`
- `SurfaceState`
- `SupervisorState`
- `SupervisorShutdownReason`
- `RuntimeOwnerMode`
- `BackgroundChannelSurface`

### `crates/daemon/src/lib.rs`

Expose the supervisor entrypoint and any lightweight CLI-adjacent helpers:

- parse high-level runtime selection
- delegate execution to `supervisor.rs`
- keep top-level CLI/business logic thin

### `crates/daemon/src/main.rs`

Add one new command surface that starts the concurrent supervisor. The command
should remain narrowly scoped to MVP runtime ownership for CLI + Telegram +
Feishu rather than attempting to solve the broader operator UX in `#217`.

### `crates/app/src/channel/mod.rs`

Keep existing channel serve logic in place. The supervisor should call existing
entrypoints or small extracted helper seams from this module rather than
rewriting Telegram or Feishu runtime behavior.

This module will need one new cooperative stop seam. The daemon runtime owner
must not rely on task abort as the normal shutdown path because channel runtime
cleanup currently happens only when the serve future returns normally.

The design should therefore add an app-layer contract such as:

- a stop token / cancellation handle passed into serve execution
- a shared shutdown future the serve loop can observe
- a small wrapper that converts daemon shutdown intent into a normal channel
  serve return path

Forceful Tokio task abort should remain only a bounded fallback when graceful
shutdown exceeds its deadline.

### `crates/app/src/channel/runtime_state.rs`

Keep existing persisted runtime state intact:

- channel-specific heartbeat files
- busy / running / stale semantics
- per-account runtime visibility

The supervisor should not fork or replace this model. If needed, it may read or
align with it, but `#301` should not redesign runtime persistence.

## Runtime Model

Represent the runtime owner inputs explicitly. Initial variants should be:

- `Telegram { account_id: Option<String> }`
- `Feishu { account_id: Option<String> }`

Represent the CLI separately as the foreground host mode in the first slice.
This avoids pretending that the current blocking stdin REPL is already a normal
supervised child.

Each child should have a small process-local lifecycle record:

- `starting`
- `running`
- `stopping`
- `stopped`
- `failed`

Minimum state payload per background child:

- surface id
- task phase
- start timestamp
- stop timestamp
- optional last error string
- optional exit reason

This state is process-local only for this phase. It is meant to support:

- deterministic logging
- supervisor-level assertions in tests
- future integration into broader operator surfaces

## Startup Flow

1. CLI command enters the supervisor entrypoint in `crates/daemon`.
2. Config is loaded once.
3. The runtime owner resolves which background channel surfaces should be
   launched.
4. A `SupervisorSpec` is created.
5. Shared shutdown and state containers are initialized.
6. One Tokio task is spawned per selected background channel.
7. Each background task wrapper:
   - marks itself `starting`
   - invokes the underlying surface runner
   - transitions to `running` after startup succeeds
   - transitions to `failed` on unexpected exit
   - transitions to `stopped` on expected shutdown
8. The foreground CLI host continues to own stdin / stdout interaction while the
   background channel tasks run concurrently in the same process.

Foreground CLI hosting must be cancellable in concurrent mode. The simplest
design is to introduce a small host seam that:

- reads stdin through a controllable adapter
- observes a shutdown notification from the runtime owner
- exits its loop cleanly when shutdown is requested or a required background
  child fails

That seam can be implemented with async stdin handling or with a blocking stdin
reader bridged into a channel, but the important contract is that the runtime
owner can terminate the CLI host coherently instead of leaving it hung on
`read_line()`.

## Shutdown Flow

Graceful shutdown is coordinated at the runtime-owner root:

1. receive Ctrl-C or another terminal shutdown trigger
2. record shutdown intent
3. signal all background channel tasks through a shared cooperative stop path
4. signal the foreground CLI host through its dedicated shutdown path
5. mark children `stopping`
6. allow app-layer serve loops to exit normally and perform explicit runtime
   tracker cleanup
7. wait for bounded task joins and foreground host exit
8. return success only when the runtime owner has shut down coherently

If any child exits unexpectedly before shutdown begins:

1. record the child failure
2. mark the runtime owner as failed
3. begin coordinated shutdown for the remaining children
4. notify the foreground CLI host to unwind and exit
5. return a summarized process-level error

This keeps `#301` operationally honest: one required child dying means the
multi-surface runtime is no longer healthy.

## Health Semantics

For this issue, "health monitoring per channel task" means:

- the supervisor knows whether each child is starting, running, stopping,
  stopped, or failed
- the supervisor records the failing child and failure text on abnormal exit
- tests and logs can observe that state deterministically

It does **not** mean a new operator-facing `status` command in this slice.

Reasoning:

- `#301` needs lifecycle truth for concurrent runtime management
- `#217` owns the broader operator-runtime UX problem
- the repo already has adjacent `channels`, `doctor`, and ACP status surfaces,
  so introducing a new status UX here would blur ownership and widen scope

## Failure Policy

This phase is intentionally fail-fast.

### Included

- fail startup if a required child cannot initialize
- fail the supervisor if a running child exits unexpectedly
- shut remaining children down when one required child fails

### Deferred

- automatic restarts
- exponential backoff
- retry budgets
- stale-child reclamation policy beyond existing channel runtime semantics

Those are real policy choices and belong in a follow-up issue rather than this
first concurrency slice.

## Session Isolation

Issue `#301` requires per-session state isolation. That needs to be explicit in
the design, not assumed.

Isolation for the first slice should mean:

- Telegram and Feishu continue to use their existing route-derived conversation
  / session identities
- the concurrent CLI host must not silently reuse the generic default session in
  supervisor mode
- the runtime owner must require or derive an explicit CLI session id when
  launched in concurrent mode

The simplest safe rule for the first slice is:

- concurrent-mode CLI always runs with an explicit session id
- channel turns continue to derive their own route/session identity
- no background child may share CLI session identity implicitly

That preserves SQLite-backed session separation without redesigning the session
store.

## Testing Strategy

### Unit tests

Add focused tests for the supervisor state machine:

- startup state transitions
- child failure propagation
- coordinated shutdown transitions
- final exit summarization

### Integration tests

Add deterministic integration tests for:

- concurrent startup of multiple surfaces
- one child failure triggering full supervisor shutdown
- Ctrl-C / shutdown path joining all children cleanly
- CLI session id staying distinct from Telegram / Feishu route-derived session
  ids in concurrent mode
- background child failure causing the foreground CLI host to exit with the
  summarized shutdown reason
- cooperative stop causing normal channel runtime cleanup instead of leaving
  active heartbeat state behind

Tests should prefer fake or controlled child runners over real network traffic
where the goal is orchestration proof rather than transport proof.

Channel-specific runtime correctness remains covered by existing channel tests.
`#301` tests should mainly prove process orchestration, lifecycle tracking, and
shutdown semantics.

## Acceptance Criteria Mapping

Issue `#301` acceptance criteria map to this design as follows:

- **Session supervisor that spawns tokio tasks per channel**
  - provided by the new daemon-owned supervisor module
- **Concurrent CLI + Telegram + Feishu in one process**
  - provided by one foreground CLI host plus supervised Telegram and Feishu
    background tasks in a single runtime owner
- **Per-session state isolation**
  - preserved by explicit CLI session identity in concurrent mode plus existing
    route-derived channel session identity
- **Graceful shutdown across all channels**
  - provided by one coordinated supervisor shutdown path
- **Health monitoring per channel task**
  - provided by process-local lifecycle state and failure recording

## Out of Scope

This design intentionally does not cover:

- Matrix integration in the first `#301` slice
- service install / daemonization UX
- new operator status rendering
- gateway route mounting
- restart / backoff / resilience policy
- cross-process supervisor persistence

## Implementation Notes

The safest implementation path is incremental:

1. add supervisor types and in-memory state transitions
2. wrap one fake child runner and prove lifecycle semantics in tests
3. extract a cancellable foreground CLI host seam for concurrent mode
4. wire in Telegram and Feishu serve children
5. add coordinated shutdown and failure propagation tests

That sequencing keeps the first risky step at the orchestration seam rather than
mixing orchestration and transport changes at once.
