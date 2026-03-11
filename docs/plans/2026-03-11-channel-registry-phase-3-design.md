# Channel Registry Phase 3 Design

**Scope**

Phase 3 adds a lightweight channel registry and readiness surface for LoongClaw.
It intentionally stops short of a full persistent run-state monitor. The goal is
to give Telegram and Feishu/Lark a shared catalog, alias normalization path, and
operator-visible readiness model before introducing more channels.

**Why This Phase Exists**

After Phase 1 reliability fixes and Phase 2 runtime/session abstraction, LoongClaw
still lacked one of OpenClaw's most important structural seams: a shared channel
directory that can answer basic questions consistently:

- what channels exist
- which aliases normalize to which surface
- which operations each channel exposes
- whether a given operation is ready, disabled, unsupported, or misconfigured

OpenClaw does this with a registry plus runtime state plumbing. LoongClaw did not.
Telegram and Feishu checks were still duplicated across daemon CLI and channel adapters.

**Reference Findings From OpenClaw**

OpenClaw separates:

- channel identity / aliases / metadata
- runtime busy-state updates
- operator-facing selection and status surfaces

The `registry.ts` layer is the keystone because it normalizes channel IDs and
prevents every caller from re-encoding channel knowledge ad hoc.

**Chosen Design**

Add a shared channel registry in the app crate with:

- `ChannelCatalogEntry`
- `ChannelCatalogOperation`
- `normalize_channel_platform(...)`
- `channel_status_snapshots(...)`

Each status snapshot reports:

- channel identity and aliases
- transport family
- compile-time availability
- config enablement
- API base URL and operator notes
- per-operation health

Per-operation health is explicit:

- `ready`
- `disabled`
- `unsupported`
- `misconfigured`

This lets Feishu model two different operational surfaces under one channel:

- `send`
- `serve`

That distinction matters because Feishu direct send only needs app credentials,
while webhook serving additionally needs allowlist and webhook verification inputs.

**Why This Is The Right Next Step**

This phase is the smallest meaningful step toward OpenClaw's runtime model:

- it removes duplicated channel readiness logic
- it gives `lark -> feishu` aliasing a first-class home
- it creates a stable metadata seam for a future run-state heartbeat
- it keeps Discord deferred until the shared operator surface exists

**Deferred Work**

This phase intentionally defers:

- persistent active-run tracking / heartbeat timestamps
- background monitor daemons
- account-scoped multi-instance registry entries
- Discord integration
- hot-reloadable external channel plugins

Those are easier and safer after registry/status semantics exist.
