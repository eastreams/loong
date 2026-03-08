# MVP Foundation Architecture

Last updated: 2026-03-08

This document defines the architectural baseline for LoongClaw MVP so future
construction, extension, and verticalization can evolve without core rewrites.

## 1) Layered Baseline

LoongClaw MVP is intentionally split into four layers:

1. `Entry Layer` (`loongclawd` CLI commands)
   - Owns process lifecycle and command argument parsing.
   - Must not embed business logic beyond argument validation.
2. `Application Layer` (`mvp/conversation/*`)
   - Owns turn orchestration: request assembly, provider call, error policy,
     and memory persistence semantics.
   - Serves as a single use-case pipeline for all channels.
3. `Adapter Layer` (`mvp/channel/*`, `mvp/provider/*`, `mvp/tools/*`, `mvp/memory/*`)
   - Owns protocol-specific I/O and translation.
   - No adapter should directly re-implement orchestration rules.
4. `Kernel Layer` (`crates/kernel`)
   - Owns policy, capability boundary, and plane orchestration invariants.

## 2) Current Contract Points

### 2.1 Conversation Contract

`ConversationOrchestrator` is the canonical turn pipeline. The conversation
domain is now split by responsibility (`mod.rs`, `runtime.rs`,
`orchestrator.rs`, `persistence.rs`, `tests.rs`), and depends on a runtime port
(`ConversationRuntime`) rather than concrete provider/memory functions directly:

- Input: `session_id`, `user_input`, `ProviderErrorMode`
- Output: final assistant text (`Ok`) or propagated provider error (`Err`)
- Guarantees:
  - user input is always injected into the outgoing message list
  - `InlineMessage` mode converts provider failure to deterministic text
  - memory persistence policy is centralized, not duplicated by channels
- extension-safe runtime injection (`handle_turn_with_runtime`) is available for
  testing and alternate backends without changing orchestrator core code

### 2.2 Channel Contract

`ChannelAdapter` defines channel integration boundary:

- `receive_batch()` returns canonical `ChannelInboundMessage` list
- `send_text()` emits output back to channel target
- channel-specific polling/webhook internals stay in adapter modules

Current first-party adapters:

- Telegram polling adapter (`mvp/channel/telegram.rs`)
- Feishu send + webhook adapter (`mvp/channel/feishu/*`)
  - `adapter.rs`: Feishu API auth/send/reply transport
  - `payload/mod.rs`: payload public surface and module wiring
  - `payload/outbound.rs`: outbound payload encode/response checks
  - `payload/inbound.rs`: inbound webhook parse/filter policy
  - `payload/crypto.rs`: encrypted webhook payload decrypt lane
  - `payload/types.rs`: inbound event/action domain types
  - `webhook.rs`: webhook state machine and dedupe/retry reply flow

### 2.3 Provider Contract

Provider layer currently exposes one stable operation:

- `request_completion(config, messages)` with retry/timeout/backoff policy
- `fetch_available_models(config)` for live model catalog discovery

Provider presets (alphabetical, OpenAI-compatible payload family):

- `anthropic`, `kimi`, `minimax`, `ollama`, `openai`, `openrouter`,
  `volcengine`, `xai`, `zai`, `zhipu`

Auth model:

- API key bearer auth (provider-specific default env fallback)
- OAuth bearer auth (`oauth_access_token` / `oauth_access_token_env`) with higher priority

Model selection model:

- explicit model name (`provider.model = "<model-id>"`)
- dynamic discovery mode (`provider.model = "auto"`) via provider models endpoint
- optional preferred ranking (`provider.preferred_models`)

Reasoning controls:

- optional `provider.reasoning_effort = none|minimal|low|medium|high|xhigh`
- serialized with compatibility negotiation (`reasoning_effort` first, fallback to
  `reasoning.effort`, and omitted when endpoint rejects both)

Compatibility negotiation:

- token limit field is negotiated per request (`max_completion_tokens` and
  `max_tokens` fallback paths)
- when `provider.model = "auto"`, incompatible endpoint-model combinations can
  trigger next-candidate fallback without hardcoded model allowlists

Design principle:

- any provider extension should preserve message contract and error taxonomy
- transport-level retry policy remains config-driven, not hardcoded per call site

Current internal composition:

- `provider/mod.rs`: public provider API and feature-gate enforcement
- `provider/policy.rs`: retry/backoff/timeout policy normalization
- `provider/transport.rs`: request headers + response body decode boundaries
- `provider/shape.rs`: response shape extraction and normalization

## 3) Feature-Flag Architecture

The daemon keeps feature slices compile-time detachable:

- channels: `channel-cli`, `channel-telegram`, `channel-feishu`
- providers: `provider-openai`, `provider-volcengine`
- runtime support: `config-toml`, `memory-sqlite`, `tool-shell`, `tool-file`

Mandatory rule for new modules:

- if a module is optional at runtime scope, it must be optional at compile-time
  via feature flags and pass matrix builds in isolation.

## 4) Extension Path (No Core Rewrite)

### Add a new channel

1. Add `mvp/channel/<name>.rs` implementing `ChannelAdapter`.
2. Reuse `ConversationOrchestrator` for turn handling.
3. Register command entry in `loongclawd` with feature-gated path.
4. Add parser/adapter unit tests + feature-slice build check.

### Add a new provider mode

1. Extend provider config schema with explicit defaults.
2. Implement request path in `mvp/provider/mod.rs` (and required submodules).
3. Keep retry and timeout semantics config-driven.
4. Add response-shape tests for both success and malformed payloads.

### Add new tools or memory backends

1. Keep command surface in core adapter stable.
2. Add backend implementation behind feature gate.
3. Preserve fallback behavior for unknown operations.

## 5) Quality Gates For Architecture Evolution

Any architecture-level change must keep all gates green:

1. `cargo fmt --all`
2. `cargo test --workspace`
3. feature-slice compile matrix (no default + representative subsets)
4. programmatic pressure gate (`benchmark-programmatic-pressure --enforce-gate`)
5. docs sync for command/config/contract changes

## 6) Known Next-Level Enhancements

1. Prebuilt binary distribution installer flow (not source-build only).
2. Provider capability descriptor matrix (provider-specific features advertised by schema).
3. Conversation runtime adapter packs (e.g. remote memory backend) with no orchestrator mutation.
4. Feishu signing key lifecycle and replay-window hardening for webhook signatures.

These items are intentionally scheduled on top of the current baseline,
not by replacing it.
