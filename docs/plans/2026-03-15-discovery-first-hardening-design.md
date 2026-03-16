# Discovery-First Hardening Design

Date: 2026-03-15
Branch: `feat/tool-discovery-architecture`
Scope: post-architecture hardening for discovery-first provider tool routing
Status: Approved for implementation

## Summary

The discovery-first runtime now exists, but the branch still needs a stronger
contract around three operational seams:

- real provider-shape end-to-end coverage across the full
  `tool.search -> follow-up provider turn -> tool.invoke` path
- lightweight telemetry that quantifies the cost and effectiveness of the
  extra provider follow-up turn
- a concise architecture note that fixes the runtime contract in one durable
  place so future refactors do not drift back toward broad static tool
  exposure

This hardening slice should stay lightweight. It should not add a database, a
new distributed state layer, or per-turn dynamic provider schema expansion.
The existing discovery-first runtime already has the correct primary shape:

- provider-visible core tools: `tool.search`, `tool.invoke`
- discoverable non-core tools behind a short-lived lease
- provider parser rewriting discoverable legacy tool names into `tool.invoke`
- coordinator follow-up loop that can request a second provider turn after
  `tool.search`

The remaining work is to prove that contract across real provider response
shapes, make its runtime cost visible, and document the invariants clearly.

## Goals

- Prove the discovery-first contract across real provider body shapes instead
  of only synthetic `ProviderTurn` fixtures.
- Quantify discovery-first behavior with minimal runtime overhead.
- Keep telemetry local and low-risk by reusing existing conversation-event
  infrastructure.
- Write one durable architecture note that describes the supported contract,
  the security boundaries, and the expected failure behavior.
- Keep implementation additive on top of PR #149 without widening the provider
  surface or introducing stateful infrastructure.

## Non-Goals

- No external DB or vector store.
- No per-tool dynamic provider schema injection after `tool.search`.
- No new cross-crate audit event type unless a real observability gap remains.
- No rewrite of the existing provider request adapters.
- No expansion of the core provider-visible tool set beyond
  `tool.search` and `tool.invoke`.

## Current Gaps

### 1. Provider-shape coverage stops at parser-only normalization

`crates/app/src/provider/shape.rs` already proves that OpenAI chat-completions,
Responses, Anthropic native content blocks, Bedrock Converse blocks, and inline
function blocks normalize discoverable tools into `tool.invoke`. That is
useful, but it still stops before the real coordinator loop.

`crates/app/src/conversation/tests.rs` does cover the follow-up provider-turn
loop, but it uses manually constructed `ProviderTurn` values rather than real
provider body shapes. That leaves a contract gap between parser correctness and
coordinator correctness.

### 2. Discovery-first runtime cost is not summarized anywhere

The coordinator already knows when a tool turn contains `tool.search`, when it
requests an extra provider follow-up turn, and whether the follow-up resolves
into a second tool call or a direct final reply. It also already carries
message-level token estimates in `ProviderTurnPreparation`.

Today none of those discovery-first signals are summarized in a single runtime
projection, so it is hard to answer:

- how often a discovery-first turn requires an extra provider round
- how many extra estimated tokens the follow-up context adds
- how often `tool.search` leads to `tool.invoke` versus a dead-end reply
- whether raw-output mode still preserves the second round

### 3. The long-lived runtime contract is not fixed in design docs

The implementation plan and design plan for 2026-03-15 explain the discovery
architecture, but they are branch-local plan artifacts, not the durable design
docs index used to document living runtime contracts.

Without a concise long-lived note, later changes can drift on:

- what counts as a provider-core tool
- whether provider parsers may emit discoverable direct tool names
- whether missing `session_id` / `turn_id` scope should be rewritten
- whether raw-output mode is allowed to bypass the follow-up provider turn
- what guarantees a lease is expected to bind

## Approaches Considered

### Approach 1: add a separate runtime telemetry module

Pros:

- can expose process-local counters similar to provider failover telemetry
- avoids extending conversation analytics parsing

Cons:

- duplicates data that already exists in persisted conversation events
- adds a second observability path for one coordinator behavior
- makes post-turn debugging harder because the metrics and the event stream
  diverge

### Approach 2: extend the existing conversation-event path

Pros:

- reuses an existing persistence and analytics mechanism
- keeps telemetry local to the coordinator behavior being measured
- produces debuggable per-turn evidence and rollup-friendly summaries
- avoids cross-crate audit surface churn

Cons:

- requires adding a new event family and analytics summarizer
- slightly increases event volume when enabled

### Approach 3: do only tests + docs, skip telemetry for now

Pros:

- smallest code change
- no runtime overhead

Cons:

- leaves token and efficiency claims unmeasured
- makes future regressions in follow-up cost harder to catch

## Decision

Adopt Approach 2.

The hardening slice should:

1. Add end-to-end tests that begin with real provider response bodies,
   normalize through `extract_provider_turn_with_scope(...)`, and then run the
   real coordinator follow-up loop.
2. Emit a dedicated discovery-first conversation event family from the
   coordinator follow-up path and add an analytics summary for it.
3. Write a design-doc note that defines the discovery-first runtime contract
   and links it from the design-doc index.

## Architecture

### Provider-shape end-to-end matrix

Add coordinator tests that exercise these source shapes:

- OpenAI chat-completions `message.tool_calls`
- Responses `output.function_call`
- Anthropic `content[].tool_use`
- Bedrock `output.message.content[].toolUse`
- inline `<function=...>` blocks inside plain assistant text

Each test should:

- start from a real provider response JSON body
- parse it with `extract_provider_turn_with_scope(...)`
- feed the resulting `ProviderTurn` into `ConversationTurnCoordinator`
- execute a real `tool.search` result against the real harness/runtime
- verify the second provider turn receives the `[tool_result]` follow-up
  context
- verify the second round can issue a discoverable tool name that rewrites to
  `tool.invoke`
- verify the final reply or raw-output result comes from the invoked tool, not
  the first-round search payload

This keeps parser correctness and coordinator correctness under one contract
without duplicating parser-only tests.

### Discovery-first telemetry

Use persisted conversation events rather than a new DB or a separate exported
metrics sink.

Add a dedicated event family, for example:

- `discovery_first_search_round`
- `discovery_first_followup_requested`
- `discovery_first_followup_result`

Each payload should stay compact and structured, carrying only the fields
needed to answer efficiency and correctness questions:

- `round`
- `search_tool_calls`
- `followup_requested`
- `raw_tool_output_requested`
- `followup_outcome`
- `followup_tool_name`
- `followup_resolved_to_tool_invoke`
- `initial_estimated_tokens`
- `followup_estimated_tokens`
- `followup_added_estimated_tokens`

The coordinator already has enough state to populate those fields from the
existing preparation and follow-up messages. Estimated tokens should remain
best-effort and use the same message estimator already used by the turn
coordinator.

Analytics should summarize:

- total search rounds seen
- total follow-up rounds requested
- total follow-up result events observed
- total raw-output follow-up rounds preserved
- total search-to-invoke hits
- average / aggregate added estimated tokens
- latest snapshot for debugging

### Durable contract note

Add `docs/design-docs/discovery-first-tool-runtime-contract.md` and link it
from `docs/design-docs/index.md`.

The note should define:

- the only provider-core tools
- the discoverable-tool boundary
- parser rewrite rules for legacy discoverable tool names
- lease scope and turn/session binding expectations
- when the coordinator must request a follow-up provider turn
- raw-output behavior
- intentionally unsupported patterns, such as dynamic per-tool schema
  reinjection and direct provider access to discoverable tools

## Testing Strategy

- Add failing coordinator tests first for provider-shape e2e coverage.
- Add failing analytics tests for discovery-first event summarization.
- Implement minimal coordinator and analytics changes to pass those tests.
- Keep existing parser-only tests unchanged except where helper reuse is
  helpful.
- Run targeted `cargo test -p loongclaw-app ...` during red-green cycles, then
  a broader verification pass before commit / push.

## Risks And Mitigations

### Event volume growth

Mitigation: emit discovery-first events only when the turn actually contains
`tool.search`, and keep payloads small.

### Test brittleness from provider-shape fixtures

Mitigation: use one concise fixture per provider family and assert only the
contractual outcomes, not incidental formatting.

### Analytics drift

Mitigation: keep the event family narrowly scoped and add direct unit tests for
the summary projection.
