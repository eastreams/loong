# Channel Runtime State Phase 4 Design

**Scope**

Phase 4 adds persisted runtime activity state for long-running channel serve
operations. It does not attempt a full OpenClaw plugin monitor port. It focuses
on the smallest runtime seam that closes the current operator gap in LoongClaw:
readiness was visible after Phase 3, but liveness was not.

**Problem Statement**

Before this phase, the `alpha-test` channel implementation could answer:

- whether Telegram or Feishu/Lark was compiled
- whether a channel was enabled
- whether the required config existed

It could not answer:

- whether a serve loop was actually running
- whether a running loop had gone stale
- whether work was actively flowing through the channel
- which process owned the current serve loop

That left LoongClaw materially behind OpenClaw's monitor layer. Operators could
see "ready" and still have no evidence that a Telegram poller or Feishu webhook
server was alive.

**Reference Findings From OpenClaw**

OpenClaw keeps three concerns separate:

- channel identity and alias normalization
- monitor lifecycle state
- operator-facing status surfaces

The important runtime pattern is not the exact storage mechanism. The important
pattern is that long-running monitors publish a small shared activity surface:

- `busy`
- `activeRuns`
- `lastRunActivityAt`

Its monitor implementations update that surface from the long-lived Telegram and
Feishu monitor lifecycles instead of from one-shot send commands. That makes the
status surface trustworthy for operators.

**Findings From LoongClaw Alpha-Test**

Compared with `origin/alpha-test`, LoongClaw still had four channel gaps:

1. `ChannelInboundMessage` and reply routing were still stringly typed.
2. Telegram and Feishu serve loops had no shared runtime tracker.
3. Feishu webhook dedupe had no failed-event release path.
4. Daemon surfaces could report readiness, but not live runtime state.

Phases 1 through 3 closed the first three structural prerequisites:

- reliable Telegram offset acknowledgement
- reliable Feishu dedupe retry semantics
- typed session/target abstractions
- shared registry and readiness snapshots

Phase 4 can therefore stay narrow and only solve runtime liveness.

**Chosen Design**

Add a persisted `ChannelOperationRuntimeTracker` with one JSON file per tracked
operation under `~/.loongclaw/channel-runtime/`.

Persisted fields:

- `running`
- `busy`
- `active_runs`
- `last_run_activity_at`
- `last_heartbeat_at`
- `pid`

Derived operator view:

- `running`
- `stale`
- `busy`
- `active_runs`
- `last_run_activity_at`
- `last_heartbeat_at`
- `pid`

Only operations marked with `tracks_runtime = true` participate. In this phase:

- Telegram `serve`
- Feishu `serve`

Feishu `send` intentionally does not publish runtime state because it is a
short-lived request path, not a monitor lifecycle.

**Why Per-Operation Files**

A single shared file would create an avoidable cross-process overwrite problem
because Telegram and Feishu serve loops can run independently. One file per
`platform + operation` keeps the write pattern simple and avoids introducing a
premature coordinator or lock server.

**Lifecycle Rules**

- tracker start writes `running=true`, `busy=false`, `active_runs=0`
- run start increments `active_runs`, sets `busy=true`, and updates timestamps
- run end decrements `active_runs`, recomputes `busy`, and updates timestamps
- background heartbeat refreshes `last_heartbeat_at`
- shutdown clears `running`, `busy`, and `active_runs`
- stale runtime is computed from heartbeat age instead of trusted blindly

**Integration Points**

- `process_channel_batch(...)` updates runtime for Telegram serve work
- `feishu_webhook_handler(...)` updates runtime around provider processing
- channel registry merges persisted runtime into serve operation snapshots
- `loongclawd channels` renders runtime lines for tracked operations

**Why This Is The Right Next Step**

This preserves the staged progression implied by OpenClaw:

- Phase 2 gave LoongClaw typed channel identities
- Phase 3 gave LoongClaw a shared registry and readiness surface
- Phase 4 adds a minimal runtime liveness seam

That is enough to support real operator diagnosis without overcommitting to a
full plugin-monitor runtime that the rest of LoongClaw does not yet have.

**Deferred Work**

This phase intentionally defers:

- account-scoped runtime keys for multi-account Telegram/Feishu
- generic event envelopes shared by all future channels
- Discord runtime integration
- websocket or push-mode Feishu monitor variants
- aggregated cross-channel supervisor state

Those should follow only after LoongClaw has a shared monitor/event seam that is
generic enough to carry Discord safely.
