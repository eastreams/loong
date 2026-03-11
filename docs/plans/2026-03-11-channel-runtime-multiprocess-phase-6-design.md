# Channel Runtime Multiprocess Phase 6 Design

**Scope**

Phase 6 hardens channel runtime persistence so multiple instances of the same
channel operation do not overwrite each other on disk.

It does not add user-visible multi-account routing yet. It fixes the lower-level
state model that multi-account routing and future Discord monitor work depend on.

**Problem Statement**

Phase 4 moved LoongClaw from "readiness only" to persisted serve runtime state,
but its first storage model was still too coarse:

- one file per `platform + operation`

That solved Telegram and Feishu overwriting each other, but it still let two
instances of the same operation trample one another:

- two `telegram-serve` processes
- two `feishu-serve` processes
- future per-account serve loops on the same machine

This is a real architectural problem because the persisted runtime surface is
supposed to be the foundation for:

- operator liveness diagnosis
- account-aware channel serve state
- eventual Discord monitor integration

If same-operation instances can overwrite one another, all three layers become
untrustworthy.

**Reference Findings From OpenClaw**

OpenClaw does not model channel runtime as one global singleton blob. Telegram,
Feishu, and Discord all resolve account- and monitor-specific runtime state
before they publish health or activity.

Its account resolution layers for Telegram, Feishu, and Discord all point in the
same direction: monitor state must be instance-aware before richer transports can
be added safely.

**Chosen Design**

Switch persisted runtime storage from:

- one file per `platform + operation`

to:

- one file per `platform + operation + pid`

Examples:

- `telegram-serve-4242.json`
- `feishu-serve-5151.json`

Loader behavior:

- scan all matching runtime files for the requested operation
- support both pid-scoped files and legacy single-file state
- prefer a currently running instance over a newer stopped instance
- otherwise prefer the freshest observed instance

This keeps the public runtime view stable while removing same-operation overwrite
hazards.

**Why This Is The Right Next Step**

LoongClaw still does not have OpenClaw-style account maps for Telegram and
Feishu, so full account-aware runtime identity would be premature at the CLI
surface. But the storage layer can and should be fixed now because:

- it removes existing correctness risk
- it preserves backward compatibility
- it creates the right seam for future account IDs without redesigning runtime
  persistence again

**Compatibility**

Phase 6 must remain backward compatible with any Phase 4 runtime files already
written on disk. Loader compatibility is therefore mandatory.

**Deferred Work**

This phase still defers:

- explicit account IDs in runtime identity
- runtime aggregation for multiple simultaneous healthy instances
- duplicate-instance warnings in `channels` or `doctor`
- Discord runtime integration

Those depend on a later phase that adds account-aware or monitor-aware runtime
identity above this pid-safe storage base.
