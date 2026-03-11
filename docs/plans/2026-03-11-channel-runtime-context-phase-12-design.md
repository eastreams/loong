# Channel Runtime Context Phase 12 Design

**Scope**

Phase 12 extracts the repeated runtime-entry preparation logic for Telegram and
Feishu/Lark into a shared channel command context and a shared serve-runtime
lifecycle wrapper.

Phase 11 already made route provenance visible inside runtime entrypoints.
What remains is structural duplication:

- load config
- resolve account
- derive route metadata
- reject disabled account
- apply runtime env
- emit fallback-route warning
- start account-scoped runtime tracker
- run serve body
- shutdown tracker

That sequence is now duplicated across Telegram and Feishu serve flows, and
partially duplicated in Feishu send.

**Problem Statement**

LoongClaw is already more advanced than `alpha-test` in multi-account channel
semantics, but the runtime entry substrate is still hand-wired per channel.

That creates three concrete risks:

- Telegram / Feishu can drift in operator-facing startup semantics
- future supervisor work has no shared execution seam to plug into
- a Discord adapter would be tempted to copy another bespoke runtime path

**Chosen Design**

Add two shared layers in `crates/app/src/channel/mod.rs`:

1. `ChannelCommandContext<R>`
   - owns `resolved_path`
   - owns loaded `LoongClawConfig`
   - owns channel-specific resolved account config
   - owns `ChannelResolvedAccountRoute`

2. `with_channel_serve_runtime(...)`
   - starts `ChannelOperationRuntimeTracker`
   - passes the tracker into the serve body
   - always attempts shutdown
   - preserves the original serve failure if serve and shutdown both fail

**Behavior**

For Telegram and Feishu:

- build a typed command context from config load + account resolution
- reject disabled accounts during context construction
- use the context to print the route notice and runtime banner fields
- wrap long-lived serve execution in the shared runtime lifecycle helper

Feishu send also uses the shared command context even though it does not start a
runtime tracker.

**Why This Is The Right Next Step**

This phase does not overreach into a full supervisor yet.

Instead, it creates the missing execution seam between:

- config/account semantics
- operator/runtime warnings
- long-lived runtime lifecycle

That seam is the minimal substrate a future supervisor and Discord adapter need.
