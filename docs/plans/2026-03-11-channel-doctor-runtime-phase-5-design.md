# Channel Doctor Runtime Phase 5 Design

**Scope**

Phase 5 upgrades `loongclaw doctor` from configuration-only channel checks to
runtime-aware diagnostics for tracked serve operations.

It does not add new channel transports. It closes the operator feedback loop
opened by Phase 4 runtime persistence.

**Problem Statement**

After Phase 4, LoongClaw could persist serve runtime state and expose it through
`loongclaw channels`, but `doctor` still treated channels as a pure readiness
problem. That left two operator workflows misaligned:

- `channels` could say a serve loop was stale or not running
- `doctor` could still report the same channel as healthy because config was valid

This split was no longer defensible once runtime state existed.

**Reference Findings From OpenClaw**

OpenClaw keeps channel configuration checks and live monitor state separate.
That separation matters because "configured correctly" and "currently healthy"
are different claims.

Discord in OpenClaw is also a strong warning against overloading config checks
with runtime meaning. Its monitor stack depends on richer runtime surfaces:

- live gateway/session ownership
- route/session state
- activity tracking
- thread binding and allowlist policy

LoongClaw is not ready for Discord until its operator surfaces can already carry
that separation cleanly for simpler channels.

**Chosen Design**

Keep the existing config-derived doctor checks, but add separate runtime checks
for serve operations whose registry entries expose `runtime`.

Mapping in this phase:

- Telegram `serve`
  - config check: `telegram channel`
  - runtime check: `telegram channel runtime`
- Feishu `serve`
  - config check: `feishu webhook verification`
  - runtime check: `feishu webhook runtime`
- Feishu `send`
  - config check only

Runtime check semantics for ready serve operations:

- `running=true` and `stale=false` -> `Pass`
- `stale=true` -> `Fail`
- `running=false` and not stale -> `Warn`
- missing runtime payload -> `Warn`

**Why Separate Checks**

Keeping config and runtime separate avoids three failure modes:

1. A stale process does not get hidden behind a green config check.
2. An intentionally stopped but correctly configured serve loop shows as a warn,
   not a fail.
3. Future channels can add runtime checks without changing the meaning of the
   existing configuration checks.

**Why This Should Happen Before Discord**

Discord in OpenClaw is not a thin transport adapter. It depends on a monitor
runtime that already knows how to distinguish:

- configured vs. connected
- connected vs. active
- active vs. stale
- per-session/thread state vs. global transport state

LoongClaw needed the same operator-level separation on Telegram and Feishu
before Discord would be architecturally safe to add.

**Deferred Work**

This phase still defers:

- doctor autofix for runtime issues
- account-scoped runtime doctor rows
- runtime-aware JSON substructure beyond the current text detail field
- Discord monitor diagnostics

Those depend on the next layer: account-aware runtime identity and a generic
channel monitor abstraction.
