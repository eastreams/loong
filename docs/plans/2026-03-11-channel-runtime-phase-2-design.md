# Channel Runtime Phase 2 Design

**Scope**

This phase extends the earlier channel reliability work with one additional abstraction layer.
It does not attempt to reproduce OpenClaw's full plugin runtime. It focuses on the smallest
runtime concepts that remove current stringly-typed channel behavior and prepare LoongClaw for
future Telegram and Feishu/Lark expansion.

**Problem Statement**

LoongClaw still models channel traffic with raw strings:

- `session_id` is a formatted string assembled independently by each adapter.
- `reply_target` is a raw string whose meaning depends on the adapter.
- Feishu and Lark are not modeled as first-class variants of the same protocol surface.

This creates two concrete problems:

1. Channel adapters must encode transport meaning in ad hoc strings, which makes later thread,
   topic, and account-aware routing harder to add safely.
2. Feishu/Lark configuration cannot cleanly represent "same channel protocol, different domain"
   the way OpenClaw does.

**Reference Findings From OpenClaw**

OpenClaw consistently separates:

- channel identity and registry metadata
- plugin runtime state
- inbound normalization
- outbound target resolution
- per-channel domain or account configuration

Its Feishu plugin explicitly treats `lark` as an alias/domain variant of the same channel instead
of a separate implementation surface.

**Chosen Design**

Introduce two small abstractions now:

1. A structured channel message surface in `crates/app/src/channel/mod.rs`
   - `ChannelPlatform`
   - `ChannelSession`
   - `ChannelOutboundTarget`
   - `ChannelOutboundTargetKind`

2. A structured Feishu domain model in `crates/app/src/config/channels.rs`
   - `FeishuDomain::{Feishu, Lark}`
   - optional `base_url`
   - `resolved_base_url()` helper

**What Changes**

`ChannelInboundMessage` will stop carrying raw `session_id` and raw `reply_target` values.
Instead it will carry:

- `session: ChannelSession`
- `reply_target: ChannelOutboundTarget`

`ChannelSession` will remain intentionally small in this phase:

- `platform`
- `conversation_id`
- `thread_id`

It will expose a deterministic `session_key()` used by the existing conversation runtime.

`ChannelOutboundTarget` will make the current target semantics explicit:

- Telegram inbound replies target a conversation chat id
- Feishu inbound replies target a reply-to message id
- Feishu direct sends target a receive id

This keeps the current runtime behavior unchanged while eliminating the current string ambiguity.

**Why This Design**

This is the narrowest design that creates real reuse:

- It improves type safety without forcing a full channel event bus rewrite.
- It provides a stable place to add thread/topic/session-scope logic later.
- It lets Feishu and Lark share one configuration model immediately.
- It avoids taking on Discord before the shared runtime seam exists.

**Alternatives Considered**

**Option 1: Keep raw strings and only document conventions**

Rejected because it preserves the exact ambiguity that caused the current divergence.

**Option 2: Full `ChannelEvent` and `OutboundEnvelope` runtime rewrite now**

Rejected for this phase because it is too wide and would mix design work with several user-facing
behavior changes at once.

**Option 3: Add Discord first and abstract later**

Rejected because Discord is the largest consumer of a shared runtime seam. Adding it before the
seam would increase divergence, not reduce it.

**Behavioral Intent**

This phase is intended to be behavior-preserving except for:

- explicit Feishu/Lark domain resolution when `domain = "lark"` is configured
- clearer target validation errors if a channel adapter receives the wrong target kind

**Deferred Work**

This phase still defers:

- Telegram webhook and callback actions
- Feishu websocket mode and account multiplexing
- Discord runtime integration
- account-scoped channel registry and hot-reload system
- fully generic `ChannelEvent` / `ChannelCommand` envelopes

Those become safer after this phase because the core identity and target types exist.
