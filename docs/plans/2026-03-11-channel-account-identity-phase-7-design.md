# Channel Account Identity Phase 7 Design

**Scope**

Phase 7 adds account-aware identity to LoongClaw's existing Telegram and
Feishu/Lark channel surface without attempting full OpenClaw-style multi-account
routing yet.

This phase is intentionally narrower than "add Discord" and intentionally
broader than "rename a runtime file":

- it introduces a resolved channel account identity model
- it threads that identity through session keys and persisted runtime state
- it makes operator surfaces show which account a runtime belongs to
- it fixes Telegram offset persistence so account state does not collide

It still does not add account maps, account selection CLI flags, or concurrent
multi-account fanout loops.

**Problem Statement**

LoongClaw's channel abstraction is now stronger than before:

- typed platform/session/target model
- readiness registry
- persisted runtime state
- runtime-aware `channels` and `doctor`
- pid-safe runtime persistence

But the current identity model is still too coarse:

- `ChannelSession` keys are only `platform + conversation [+ thread]`
- runtime files are only `platform + operation [+ pid]`
- Telegram offsets are stored in one global `telegram.offset`

That means LoongClaw still cannot distinguish:

- one Telegram bot from another
- one Feishu app from another
- one Feishu account on `feishu` from another on `lark`

OpenClaw's Telegram, Feishu, and Discord layers all resolve account-aware
monitor state before they publish runtime health. LoongClaw does not need the
full OpenClaw stack yet, but it does need the same identity seam if it wants to
avoid collisions and eventually support Discord correctly.

**Reference Findings From OpenClaw**

OpenClaw's account modules point to the same architectural rule across
Telegram, Feishu, and Discord:

- account resolution must happen before runtime publication
- monitor lifecycle is account-scoped, not just platform-scoped
- session/routing state depends on that account identity

Its Discord support is not "just another adapter". It sits on top of:

- account resolution
- runtime plumbing
- monitor lifecycle
- routing/session policies

LoongClaw should therefore finish account-aware identity first instead of
pretending Discord can be bolted on safely right now.

**Chosen Design**

Introduce a resolved `ChannelAccountIdentity` for Telegram and Feishu/Lark with
three properties:

- `id`: stable machine key used in persisted files and session keys
- `label`: human-readable identity shown in CLI and doctor output
- `source`: how the identity was resolved

Resolution rules:

1. Prefer explicit config `account_id` if the operator sets one.
2. Otherwise derive a stable non-secret identity from the configured
   credentials:
   - Telegram: `bot_<bot_id>` from the token prefix before `:`
   - Feishu/Lark: `<domain>_<app_id>` from domain + app id
3. If credentials are not yet available, fall back to `default`.

This gives LoongClaw a future-compatible account seam immediately while keeping
today's single-account config shape.

**Config Changes**

Add optional `account_id` to:

- `telegram`
- `feishu`

This is not multi-account config yet. It is a stable override for persistence
and operator visibility.

**Runtime Persistence Changes**

Persist runtime by:

- platform
- operation
- account identity
- pid

Example file names:

- `telegram-serve-bot_123456-4242.json`
- `feishu-serve-lark_cli_a1b2c3-5151.json`

Loader behavior:

- prefer account-aware files for the requested account
- remain backward compatible with old files that only use
  `platform + operation [+ pid]`
- if no account-aware file exists, still accept legacy runtime files

This keeps older installations readable while moving new writes onto the
correcter identity model.

**Telegram Offset Changes**

Telegram offset persistence must move from a global singleton file to an
account-scoped file:

- new path pattern: `telegram-offsets/<account_id>.offset`
- legacy fallback: if the new file is absent, still read `telegram.offset`

This change matters as much as runtime persistence because offset state is part
of channel identity, not just transport mechanics.

**Session Key Changes**

`ChannelSession` should include account identity so conversation memory is
scoped by both platform and serving account.

Example:

- old: `telegram:123`
- new: `telegram:bot_123456:123`

This prevents two different bots or apps from sharing the same conversation
window just because the chat ID happens to match.

**Operator Surface Changes**

Expose resolved account identity in:

- `channels` text rendering
- `channels --json`
- `doctor` runtime detail strings
- channel readiness notes

This does not yet mean multiple account rows. It means the current row tells the
operator exactly which account the runtime view belongs to.

**Why This Is The Right Next Step**

This phase solves the highest-value remaining structural gap before Discord:

- runtime state becomes account-aware
- session memory becomes account-aware
- Telegram offset persistence becomes account-aware
- operator diagnostics gain explicit identity context

That is enough to make the current single-account surface safer and enough to
prepare the next layer of work:

- multi-account config maps
- per-account registry aggregation
- shared monitor abstractions
- eventual Discord integration

**Compatibility**

Phase 7 should preserve these compat rules:

- old runtime files remain readable
- old Telegram offset file remains readable as fallback
- configs without `account_id` continue to work

Behavior that intentionally changes:

- new session keys become account-scoped
- new runtime writes become account-scoped
- new Telegram offset writes become account-scoped

**Deferred Work**

Phase 7 still defers:

- full `accounts` maps for Telegram and Feishu
- default-account selection semantics
- CLI account-selection flags
- runtime aggregation across multiple configured accounts
- generic channel monitor trait
- Discord channel implementation
