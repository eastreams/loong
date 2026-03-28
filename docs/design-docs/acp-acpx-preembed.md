# ACP / ACPX Pre-Embed Notes

Date: 2026-03-11

## Why this document exists

LoongClaw already introduced two future-facing abstractions that can be confused if the boundary is
not written down early:

- `conversation::ContextEngine`
- `acp::*` control-plane scaffolding

OpenClaw's latest ACP architecture makes the split explicit:

- `context engine` is about context assembly, memory windows, compaction, and prompt shaping
- `ACP runtime` is about external harness session lifecycle, turn execution, runtime controls, and
  persistent bindings
- `ACPX` is not "ACP itself"; it is one ACP runtime backend/plugin
- `openclaw acp` CLI bridge is yet another surface: an ACP-speaking bridge in front of the Gateway

If LoongClaw collapses these layers too early, future ACP/ACPX integration will require a large
refactor across config, manager, channel routing, and daemon observability.

For the outer localhost product surface that should consume ACP without
replacing it, see [Local Product Control Plane](local-product-control-plane.md).

## OpenClaw design signals that matter

The following patterns appear consistently in OpenClaw upstream:

- Context engine is no longer just an `assemble/compact` hook. Upstream now exposes a lifecycle
  contract that includes `bootstrap`, `ingest`, `assemble`, `compact`, `afterTurn`,
  `prepareSubagentSpawn`, and `onSubagentEnded`.
- ACP runtime APIs are session-handle-aware, not only `session_key` string based.
- ACP runtime backends are shared runtime instances, not disposable per-call adapters.
- ACP session manager is a real control plane:
  - per-session actor serialization
  - runtime cache / idle eviction
  - active turn bookkeeping
  - runtime controls (`set_mode`, `set_config_option`, `status`)
  - persistent bindings from conversation identity to ACP session key
- ACPX-specific configuration is backend-local, not pushed into global ACP config.
- Bridge mode (`openclaw acp`) is intentionally distinct from backend mode (`acpx` runtime plugin).

## LoongClaw invariants after this patch

These are the architecture invariants we now want to preserve:

1. ACP must not be modeled as a `process_stdio` special case.
2. ACP must not be absorbed into `provider::*`.
3. ACP must not be absorbed into `conversation::ContextEngine`.
4. ACP backends must be able to read runtime config at call time.
5. ACP backends must receive stable session handles for turn execution.
6. ACP registry must preserve shared backend instances so backend-local runtime state can exist.
7. Conversation identity must be able to bind to an ACP session before channel-specific ACP routing
   is fully implemented.
8. Kernel/spec/plugin bridge taxonomy must not collapse `acp bridge` and `acpx/runtime backend`
   into the same bridge kind.

## What changed in LoongClaw

### 1. ACP backend trait is now config-aware and handle-aware

The ACP backend trait now receives:

- `&LoongClawConfig` on backend calls
- `&AcpSessionHandle` for `run_turn`

This is required for any real ACPX-style runtime backend because command path, expected version,
permission defaults, MCP injection policy, working directory policy, and backend-local session ids
cannot be reconstructed from `session_key` alone.

### 2. ACP registry now returns shared backend instances

The ACP registry previously created fresh backend objects on each resolve. That was acceptable only
for stateless placeholder backends.

It now stores shared backend instances (`Arc<dyn AcpRuntimeBackend>`), which preserves the ability
to add:

- runtime caches
- health state
- connection pools
- process supervisor state
- backend-local cancellation state

without changing the registry contract again later.

### 3. ACP session metadata now models control-plane state instead of only transport identity

Persisted ACP session metadata now includes:

- `conversation_id`
- `last_activity_ms`
- `last_error`

This gives LoongClaw the minimum persistence needed for:

- persistent conversation bindings
- idle TTL cleanup
- richer session observability
- later channel-specific ACP routing

### 4. Session store now supports legacy conversation lookup plus typed route binding lookup

The ACP session store no longer treats `conversation_id` as the only reusable binding identity.

It now persists both:

- legacy `conversation_id`
- structured `binding_route_session_id` plus normalized channel/account/conversation/thread scope

That means the ACP control plane can now resolve by:

- `session_key`
- `conversation_id`
- `binding_route_session_id`

This is the more important pre-embed step toward OpenClaw-style persistent bindings. With
`acp.bindings_enabled=true`, a conversation can still be rebound to the existing ACP session even
if the caller proposes a different `session_key`, but now a typed route can remain the canonical
binding identity even when the caller uses an opaque compatibility `conversation_id`.

### 5. ACP session manager gained real control-plane behavior

The Rust ACP manager now performs:

- conversation binding reuse when `bindings_enabled=true`
- structured route binding reuse when `bindings_enabled=true` and the bootstrap metadata carries
  `route_session_id`
- active turn tracking
- per-session actor serialization keyed by structured route identity first, then legacy
  conversation identity / session key
- queued-turn depth projection into session status
- runtime control serialization for:
  - `status`
  - `set_mode`
  - `set_config_option`
  - `close`
- active-turn cancellation preemption via a dedicated abort handle plus immediate backend `cancel`
  dispatch, while non-running sessions still use the serialized control path
- `max_concurrent_sessions` enforcement
- idle TTL session cleanup
- `last_error` persistence on failed turns / failed controls
- backend doctor dispatch

This is still lighter than OpenClaw's full actor system and runtime cache, but the most disruptive
semantic gap is now closed: ACP turn execution and core runtime controls already share the same
session actor seam, so later streaming / abort work does not require another manager-shape rewrite.

### 6. ACPX backend-local config now lives under `[acp.backends.acpx]`

ACPX-specific config is now modeled under:

```toml
[acp.backends.acpx]
command = "/usr/local/bin/acpx"
expected_version = "0.1.16"
cwd = "/workspace/repo"
permission_mode = "approve-reads"
non_interactive_permissions = "fail"
timeout_seconds = 45.5
queue_owner_ttl_seconds = 0.25

[acp.backends.acpx.mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/workspace/repo"]
```

This mirrors the shape of OpenClaw's ACPX plugin config and avoids polluting the global `[acp]`
section with backend-specific knobs.

### 7. ACPX runtime adapter is now real, but deliberately narrow

The Rust `acpx` backend is no longer a doctor-only probe. It now performs:

- `sessions ensure` with `sessions new` fallback when ensure returns no identifiers
- handle-state encoding inside `runtime_session_name`
- prompt turn execution via `acpx prompt --file -`
- line-by-line JSON event parsing with an internal event-sink seam, so the backend can emit runtime
  events incrementally even though current callers still consume an aggregated final result
- local prompt abort support for running turns, returning a normalized cancelled stop reason when the
  active turn is preempted
- backend-local MCP proxy agent-command injection via `--agent <wrapped-command>` when
  `bootstrap.mcp_servers` selects entries configured under `[acp.backends.acpx.mcp_servers]`
- aggregated final output / usage projection from streamed JSON-line events
- runtime control commands:
  - `status`
  - `set-mode`
  - `set`
  - `cancel`
  - `sessions close`
- backend-local config validation for permission mode / non-interactive policy / timeouts

This is intentionally still narrower than OpenClaw upstream:

- LoongClaw now has a backend-local streaming/event seam and attached active-turn abort, but channel,
  CLI, and daemon callers still consume aggregated `AcpTurnResult` rather than a public
  stream-oriented runtime API
- MCP proxy injection currently uses an embedded Node wrapper script and only activates when the
  caller explicitly requests configured `bootstrap.mcp_servers`
- runtime status is normalized into LoongClaw's current `AcpSessionStatus`, not OpenClaw's richer
  runtime observability surface

### 8. ACP doctor, status, and observability surfaces are now explicit

`loongclawd acp-doctor` now exposes backend readiness diagnostics for the selected or requested ACP
backend. `loongclawd acp-status` resolves live status by either:

- `--session <session_key>`
- `--conversation-id <conversation_id>`
- `--route-session-id <route_session_id>`

`loongclawd acp-dispatch` now evaluates the ACP dispatch decision surface directly for either:

- a raw `session_id`
- or a typed `channel/account/conversation/thread` scope

That dispatch surface now also projects the predicted automatic ACP origin when routing is allowed:

- `automatic_agent_prefixed`
- `automatic_dispatch`

`loongclawd acp-observability` now exposes a shared-manager control-plane snapshot with:

- active tracked sessions
- bound vs unbound session counts
- activation-origin rollups (`explicit_request`, `automatic_agent_prefixed`,
  `automatic_dispatch`)
- backend distribution rollups
- actor/control queue depth plus waiting operations
- turn queue depth / active turns
- turn success/failure counters
- aggregate latency metrics
- process-local error counters
- idle-eviction counters

At the moment:

- `planning_stub` returns a placeholder diagnostic
- `acpx` exposes command/version/config readiness diagnostics, including invalid backend-local
  config states, MCP proxy policy/readiness, and missing working-directory failures
- `acp-status` projects persisted conversation bindings and queued-turn depth through the shared ACP
  manager; when a session is busy it now returns manager-local fallback status instead of racing the
  backend, preserves explicit repair states such as `cancelling` while an active turn is draining,
  and when a session is idle it serializes backend status inspection behind the same session actor
  as turns and control operations, so operator visibility does not depend on provider/runtime
  internals. Session/status projections now also surface `binding_route_session_id` so typed ACP
  bindings are operator-visible instead of hidden behind legacy labels
- `acp-dispatch` exposes the canonical dispatch decision and reason (`allowed`,
  `dispatch_disabled`, `channel_not_allowed`, `account_not_allowed`, `thread_required`,
  `root_conversation_required`, `agent_prefix_required`) with the normalized route target, so ACP
  policy debugging no longer depends on hand-reconstructing session-key parsing rules
- `acp-observability` gives LoongClaw an OpenClaw-like control-plane snapshot without forcing ACPX
  runtime streaming/cache work into the current patch

This creates an operator-facing validation path before full ACPX runtime execution is implemented.

### 9. Channel / CLI turns now route through ACP via explicit dispatch policy

`ConversationTurnCoordinator` now treats ACP as a first-class runtime path:

- when `acp.enabled = false`, turns continue down the existing provider path
- when `acp.enabled = true`, normal turns still stay on the existing provider/context-engine path
  unless ACP is explicitly requested or `[acp.dispatch]` allows the session
- when a caller explicitly requests ACP for a turn, the turn enters the shared ACP control-plane
  manager even if automatic dispatch is disabled; this matches OpenClaw's `runtime: "acp"` /
  bound-session model more closely than an always-on global ACP switch
- when `acp.enabled = true` but `[acp.dispatch].enabled = false`, the ACP control plane remains
  available while normal conversation turns stay on the provider/context-engine path; only explicit
  ACP requests still enter ACP
- daemon CLI chat now exposes an explicit `--acp` switch for the same semantics: it forces chat
  turns onto the ACP path for that session without changing the global automatic dispatch baseline
- default LoongClaw dispatch now mirrors the safer OpenClaw expectation more closely:
  `[acp.dispatch].conversation_routing = "agent_prefixed_only"` unless a caller opts into a wider
  automatic routing policy
- when `[acp.dispatch].conversation_routing = "agent_prefixed_only"`, only explicit
  `agent:<id>:` sessions default into ACP; non-prefixed sessions continue on the provider path
- when `[acp.dispatch].allowed_channels` is non-empty, only sessions whose underlying channel route
  matches the allowlist can default into ACP
- when `[acp.dispatch].allowed_account_ids` is non-empty, only sessions whose structured account
  scope matches the normalized allowlist can default into ACP
- when `[acp.dispatch].thread_routing = "thread_only"|"root_only"`, ACP dispatch can explicitly
  gate thread-bound versus root-bound conversations without inventing new session-key heuristics
- channel ingress now builds a structured `ConversationSessionAddress` with explicit
  `channel/account/conversation/thread` fields, and ACP dispatch prefers that typed address before
  falling back to legacy `session_id` parsing

The current route derivation is intentionally explicit:

- `conversation_id = <original session_id>`
- `session_key = agent:<selected_agent>:<session_id>` unless the session id is already
  agent-prefixed
- `binding = AcpSessionBindingScope { route_session_id, channel/account/conversation/thread }`
  when the route resolves to a channel-scoped identity, so ACP bootstrap can carry typed binding
  state directly instead of forcing manager/store logic to re-parse only metadata
- `selected_agent` comes from ACP control-plane policy (`default_agent` plus `allowed_agents`),
  not channel-specific heuristics inside the backend
- dispatch/channel metadata is derived from the underlying route session id, so
  `agent:<id>:feishu:...` keeps `channel=feishu` instead of accidentally collapsing to `channel=agent`
- route metadata can now also persist `channel_account_id`, `channel_conversation_id`, and
  `channel_thread_id` when the caller provides a typed channel address, so future OpenClaw-style
  account/thread bindings do not require another turn-entry rewrite
- ACP manager/store now lift that metadata into a first-class `AcpSessionBindingScope`, so future
  account/thread binding policy no longer depends on re-parsing ad hoc metadata or mutating the
  public ACP bootstrap surface again
- `AcpSessionBootstrap` now also carries optional typed `binding` explicitly; metadata keeps the
  same values for compatibility and observability, but control-plane binding is no longer metadata-only
- dispatch evaluation is now internally modeled as a decision with an explicit reason
  (`dispatch_disabled`, `channel_not_allowed`, `account_not_allowed`, `thread_required`,
  `root_conversation_required`, `agent_prefix_required`), which gives daemon/diagnostic surfaces a
  stable explanation seam later without rewriting the routing core

This matters because it finishes the main pre-embed goal: channel identity now feeds ACP bindings
without pushing ACP semantics down into `conversation::ContextEngine`.

The important architectural change is that LoongClaw no longer overloads a single flag for two
different meanings:

- `acp.enabled` = ACP control plane exists and can be used
- `acp.dispatch.*` = which conversation turns default into that control plane automatically
- `acp::AcpConversationTurnOptions.routing_intent = explicit` = caller explicitly wants ACP for this
  turn even when automatic dispatch would stay on the provider path
- `AcpRoutingOrigin` / `activation_origin` = ACP records whether a routed turn/session exists
  because of an explicit request, an `agent:<id>:` prefixed route, or generic automatic dispatch

This mirrors OpenClaw's documented ACP shape more closely and removes the next likely refactor
trap for mixed provider/ACP operation or future thread-binding policy.

### 10. Shared ACP manager instances now survive across turns per memory store

LoongClaw now reuses a shared `AcpSessionManager` keyed by the configured sqlite memory path.

This prevents the new ACP route from degenerating into:

- \"new manager every turn\"
- duplicated `ensure_session(...)` calls
- lost process-local active-turn tracking

It is still lighter than OpenClaw's full runtime cache / observability stack, but it preserves the
correct control-plane shape for future upgrades.

### 11. Kernel/spec plugin taxonomy now distinguishes ACP bridge and ACP runtime surfaces

The kernel/spec plugin bridge taxonomy now treats these as different surfaces:

- `acp_bridge`: a bridge/gateway-facing ACP surface such as `openclaw acp`
- `acp_runtime`: a session-aware ACP runtime/backend surface such as an ACPX-style runtime plugin

This split is intentionally pre-embedded before bridge-mode parity is fully implemented.

Without it, plugin IR, bridge support matrices, bootstrap hints, runtime evidence, and future
policy gates would keep treating bridge mode and runtime-backend mode as the same abstraction,
which would force a more invasive refactor later when both surfaces exist at once.

The bootstrap policy layer now also preserves this split:

- `allow_acp_bridge_auto_apply`
- `allow_acp_runtime_auto_apply`

This keeps future bridge-mode rollout governance separate from ACP runtime-backend rollout, instead
of forcing both surfaces through a single ACP auto-apply gate.

### 12. ContextEngine lifecycle seam now pre-embeds OpenClaw-style bootstrap and ingest hooks

LoongClaw's `ConversationContextEngine` was already split away from provider assembly and ACP
control-plane logic. This patch extends that seam with two additional default no-op hooks:

- `bootstrap(...)`
- `ingest(...)`

Together with the already reserved hooks:

- `assemble_context(...)`
- `compact_context(...)`
- `after_turn(...)`
- `prepare_subagent_spawn(...)`
- `on_subagent_ended(...)`

this gives LoongClaw an internal lifecycle surface that is materially closer to OpenClaw's current
`ContextEngine` plugin slot without changing current behavior.

The important boundary is preserved:

- these hooks are used only on the normal conversation/provider path
- ACP-routed turns still bypass the conversation context engine and execute through the ACP control
  plane
- the hooks are currently pre-embed seams, not a new storage authority

That means future context engines can own bootstrap/import or sidecar ingestion work without
forcing a trait-breaking refactor later, while ACP/ACPX remains a separate runtime concern.

The runtime side now mirrors that seam as well:

- `ConversationRuntime::prepare_subagent_spawn(...)`
- `ConversationRuntime::on_subagent_ended(...)`

with `DefaultConversationRuntime` delegating those calls straight into the selected
`ConversationContextEngine`. That matters because future subagent-aware context engines can now be
introduced without changing the runtime trait again.

### 13. ACP runtime events now externalize past the backend seam without changing the turn API

LoongClaw now extends the ACP control-plane outward one more layer while preserving the same
aggregated `AcpTurnResult` contract for current callers.

The new pieces are:

- `AcpSessionManager::run_turn_with_sink(...)` as the formal manager seam for streamed ACP turn
  events
- `ConversationTurnCoordinator::*acp_event_sink(...)` entrypoints as the conversation turn-coordinator
  seam for live ACP runtime event subscribers, so callers can observe ACP runtime events directly
  without reaching into manager/backend internals
- conversation-path persistence of structured `acp_turn_event` and `acp_turn_final` records into
  the existing `conversation_event` storage lane
- explicit `agent_id` persistence alongside `session_key`, so per-agent observability and future
  binding diagnostics do not depend on parsing route strings
- explicit `routing_intent` persistence (`automatic` vs `explicit`), so operators can tell whether
  an ACP turn was entered by default dispatch policy or by an explicit caller request
- explicit `routing_origin` persistence (`explicit_request`, `automatic_agent_prefixed`,
  `automatic_dispatch`), so operators can tell which ACP-only entry condition caused the routed
  turn
- session-level `activation_origin` persistence in the ACP store/status plane, so `acp-status`,
  `list-acp-sessions`, and `acp-observability` can explain why an ACP session exists without
  inferring from backend-local runtime state
- opt-in runtime-event persistence via `[acp].emit_runtime_events = true`, so history growth stays
  backward compatible by default
- `loongclawd acp-event-summary` as the first daemon-facing summary surface over those persisted
  runtime-event records

This is intentionally narrower than OpenClaw's public live-stream runtime APIs, but it matters for
architecture because it breaks the previous trap where runtime events existed only inside ACPX
backend internals. Now manager, conversation persistence, and daemon observability all share a
stable event shape without collapsing ACP into the context engine or daemon directly into backend
details. It also means LoongClaw now has a stable conversation-layer live-event seam even before
daemon/channel default consumers are upgraded to use it.

### 14. ACP agent selection is now explicit control-plane policy rather than backend inference

LoongClaw previously still had one unstable seam: the selected ACP agent could be inferred from
session-key prefixes or backend-local heuristics.

That shape does not scale. It makes route derivation, backend execution, and observability disagree
about who the logical ACP agent actually is, and it quietly widens the security surface by letting
arbitrary agent labels leak through implicit paths.

The control plane now carries that policy explicitly:

- `[acp] default_agent`
- `[acp] allowed_agents`
- `[acp.dispatch] enabled`
- `[acp.dispatch] conversation_routing`
- `[acp.dispatch] allowed_channels`

The invariants are:

- default agent resolution happens once at the ACP control-plane layer
- ACP dispatch policy is evaluated before the conversation runtime decides whether to enter ACP at
  all
- channel filtering applies to the parsed underlying conversation route, not to the raw
  agent-prefixed wrapper alone
- non-prefixed conversation routes derive `agent:<default_agent>:<conversation_id>`
- prefixed session keys are accepted only when the embedded agent is in `allowed_agents`
- ACPX now validates metadata/session-key agent consistency instead of silently picking one side

This keeps agent selection as ACP policy, not ACPX implementation detail, and aligns better with
OpenClaw's documented ACP agent-management direction.

### 15. ACP-owned turn options now front-load per-turn extensibility without API sprawl

One remaining pre-embed risk was API shape drift in the conversation turn-coordinator layer.

After adding the public ACP live event sink, the next obvious trap would have been to keep growing
 `ConversationTurnCoordinator` with one more variant per new ACP concern:

- one method for live runtime events
- another for extra bootstrap MCP selections
- later more for provenance / receipts / workflow-local runtime hints

That does not scale. It would force channel, daemon, and chat callers to keep chasing a widening
set of near-duplicate entrypoints.

LoongClaw now introduces an ACP-owned turn options object:

- `acp::AcpConversationTurnOptions`
  - `event_sink`
  - `additional_bootstrap_mcp_servers`
  - `routing_intent`
  - `working_directory`
  - `provenance`

The important part is not the current field count. The important part is that the ACP layer now has
one stable per-turn option seam where future OpenClaw-like control-plane hints can be added
without re-breaking every caller surface or pretending those hints belong to the context engine.

The current behavior is intentionally narrow:

- config-level baseline still comes from `[acp.dispatch].bootstrap_mcp_servers`
- per-turn callers can add extra backend-local MCP server names on top of that baseline
- normalization/deduplication still flows through ACP dispatch config rules
- ACP-routed turns still bypass context-engine lifecycle exactly as before

That preserves the architecture invariant that ACP remains a separate runtime/control-plane concern,
while still creating a real extensibility slot for future provenance, receipts, or richer ACPX
runtime hints.

The other half of the change is just as important:

- `acp::prepare_acp_conversation_turn_for_address(...)` is now the single place that converts
  conversation identity plus per-turn ACP options into:
  - derived ACP route
  - resolved `routing_origin`
  - normalized bootstrap metadata
  - normalized turn request metadata

That means `conversation::ConversationTurnCoordinator` no longer owns ACP metadata assembly. It
only decides whether to enter ACP and then hands execution to the ACP control plane.

### 16. Daemon chat is now a real consumer of ACP live events and per-turn bootstrap MCP selection

The new conversation-layer seam is no longer test-only.

`loongclawd chat` now consumes it directly:

- `--acp`
- `--acp-event-stream`
- repeated `--acp-bootstrap-mcp-server <name>`
- `--acp-cwd <path>`

This matters for architecture more than for operator ergonomics:

- it validates that live ACP runtime events can leave backend internals and reach a real daemon
  surface without forcing persistence-on
- it validates that extra per-turn MCP bootstrap selection can be layered on top of the ACP
  dispatch baseline without mutating global config or leaking ACPX-specific config into top-level
  `[acp]`
- it gives LoongClaw a first operator-facing ACP turn override surface that is still routed through
  the conversation turn-coordinator abstraction, not around it

This is still intentionally smaller than OpenClaw's broader ACP bridge/runtime surfaces, but it
removes two future refactor cliffs:

1. daemon/chat no longer needs a new side-channel when richer ACP runtime streaming is exposed
2. per-workflow MCP selection no longer requires a new coordinator signature every time a caller
   wants ACP-local bootstrap hints

### 17. Channel/account ACP bootstrap policy now resolves through runtime account identity

One more future refactor cliff sat in the channel layer.

Channel ingress already normalizes session identity into:

- platform
- runtime account identity
- conversation/thread scope

But account selection config is still authored in terms of configured account ids / labels, while
actual inbound sessions often carry derived runtime account identities such as:

- Telegram: `ops-bot` or `bot_123456`
- Feishu/Lark: `lark_cli_a1b2c3`

If LoongClaw kept treating those as unrelated namespaces, future ACP policy or per-account runtime
selection would either:

- duplicate channel-specific lookup rules at every caller, or
- force later invasive changes to `ChannelSession`

LoongClaw now resolves channel ACP additions through the runtime account identity itself:

- Telegram and Feishu config both expose a small `acp` sub-config
- `process_inbound_with_provider(...)` resolves the inbound session account back to the matching
  configured account or derived runtime account
- the resolved channel/account ACP additions are then forwarded into
  `acp::AcpConversationTurnOptions`

Current scope is deliberately narrow:

- `channel.acp.bootstrap_mcp_servers`
- account-local override support
- additive merge with the global ACP dispatch baseline through the shared dispatch normalization
  path

That means channel-specific ACP bootstrap policy now lives above ACPX backend config, but below the
conversation turn-coordinator seam, which is exactly where future per-channel workflow policy belongs.

### 18. Channel status snapshots now expose ACP bootstrap MCP presets for operator visibility

Hidden routing policy is bad architecture because it creates operational blind spots.

Once channel/account ACP bootstrap additions existed, LoongClaw needed an operator-facing surface to
show them without forcing users to reverse-engineer merged config state.

Channel registry snapshots now add:

- `acp_bootstrap_mcp_servers=...`

to per-account channel notes when such presets are configured.

This is intentionally simple, but it matters because:

- channel/account ACP routing policy is now observable alongside account identity and default-route
  source
- future debugging of ACP bindings, route mismatches, or backend MCP injection no longer depends on
  remembering hidden config inheritance
- LoongClaw's operator-facing status surfaces stay aligned with the control-plane-first design
  direction instead of burying ACP policy inside backend-local internals

### 19. ACP-owned turn provenance now has a first-class seam

One more refactor trap remained after adding `AcpConversationTurnOptions`.

Per-turn ACP callers already needed places to carry:

- runtime event consumers
- additive bootstrap MCP selections

The next likely pressure points were always going to be:

- `trace_id`
- channel source message identity
- delivery receipt / ack cursor

If those were introduced ad hoc later, LoongClaw would almost certainly repeat the same mistake:
grow a fresh coordinator entrypoint, or worse, push channel-delivery semantics directly into ACPX
backend code.

LoongClaw now makes that seam explicit inside `acp::*`:

- `acp::AcpTurnProvenance`
  - `trace_id`
  - `source_message_id`
  - `ack_cursor`
- `AcpConversationTurnOptions` now carries that provenance alongside the existing event sink and
  bootstrap MCP additions

Current behavior is intentionally conservative:

- provenance is normalized into reserved ACP request metadata keys at the conversation/control-plane
  boundary
- the same fields are projected into persisted ACP runtime-event records so later observability,
  receipts, and trace tooling can reuse one stable shape
- channel ingress now forwards existing inbound delivery identifiers through this seam instead of
  inventing channel-local ACP special cases

This matters because it preserves the right ownership:

- `channel::*` owns raw delivery semantics
- `conversation::*` owns turn orchestration and the default provider/context-engine path
- `acp::*` owns per-turn ACP intent/provenance, route preparation, runtime/control-plane transport,
  and persistence
- `acpx` remains only a runtime backend that may consume the metadata, not the place where those
  semantics are invented

That is closer to OpenClaw's direction as well. Upstream ACP work clearly distinguishes:

- Gateway-backed bridge session metadata
- control-plane session/runtime management
- context-engine lifecycle

Adding a first-class provenance seam now means future ACP receipts / trace correlation / workflow
audits can attach at the conversation/control-plane layer without reopening the coordinator
signature yet again.

### 20. ACP working-directory routing is now pre-embedded as a runtime hint instead of a backend quirk

There was one more hidden refactor cliff in the ACP path:

- `AcpSessionBootstrap` already had `working_directory`
- `AcpTurnRequest` already had `working_directory`
- `acpx` backend already knew how to honor it
- but the conversation turn-coordinator path still hard-coded both to `None`

That meant any future requirement for:

- per-channel workspace routing
- per-workflow repo selection
- ACP bridge/runtime parity around cwd-like execution hints

would have forced yet another coordinator signature change or a backend-local special case.

LoongClaw now lifts that hint into the same stable conversation-layer seam:

- `AcpConversationTurnOptions.working_directory`

and wires it through:

- ACP session bootstrap when a session is first established
- ACP turn requests for per-turn override semantics
- channel/account ACP config via `channel.acp.working_directory`
- daemon chat via `--acp-cwd`
- channel status notes via `acp_working_directory=...`

This is the correct ownership boundary:

- channel/account/workflow layers choose the runtime workspace hint
- conversation turn-coordinator carries it as a per-turn ACP concern
- ACP control-plane transports it
- ACPX consumes it as one backend implementation detail

This only applies after a turn has already entered the ACP path. A dispatch-level working directory
does not make ACP globally always-on; it remains a baseline runtime hint for ACP-routed turns only,
which is the same architectural boundary OpenClaw keeps between context/session routing and ACP
runtime execution.

The important part is that cwd selection still does **not** live in backend-local ACPX config only.
Backend-local config remains the place for runtime binary / permissions / timeouts / MCP proxy
details. Conversation-scoped workspace routing now has a stable abstraction above that layer.

### 21. Daemon/operator provenance contract now distinguishes prediction, activation, and turn facts

Another future refactor trap sat in the operator surface.

Even after introducing `routing_intent`, `routing_origin`, and `activation_origin`, daemon-facing
JSON could still drift into an ambiguous shape where every command exposed some flavor of `origin`
but operators had to infer which layer the field belonged to:

- dispatch policy prediction
- persisted session activation reason
- last executed turn routing fact

LoongClaw now makes that distinction explicit in daemon JSON by adding a structured `provenance`
object to each relevant ACP surface while keeping the existing flat fields for backward
compatibility:

- `list-acp-sessions` / `acp-status`
  - `provenance.surface = session_activation`
  - `provenance.activation_origin = ...`
- `acp-dispatch`
  - `provenance.surface = dispatch_prediction`
  - `provenance.automatic_routing_origin = ...`
- `acp-event-summary`
  - `provenance.surface = turn_execution`
  - `provenance.last_routing_intent = ...`
  - `provenance.last_routing_origin = ...`
  - `provenance.routing_intent_counts = ...`
  - `provenance.routing_origin_counts = ...`
- `acp-observability`
  - `sessions.provenance.surface = session_activation_aggregate`
  - `sessions.provenance.activation_origin_counts = ...`

This matters because it turns the operator contract into a layered explanation model:

- `acp-dispatch` answers: "if automatic ACP policy evaluates this route now, what would happen?"
- `acp-status` / `list-acp-sessions` answer: "why does this ACP session exist?"
- `acp-event-summary` answers: "why did the routed ACP turn(s) happen?"

That is the right shape for future OpenClaw-style ACP bridge/runtime growth as well. Bridge-facing
policy/debugging can expand without corrupting the meaning of session activation or executed-turn
observability, because the operator surface already reserves separate semantic layers for each.

### 22. ACP runtime-event ownership is now split between record shape and storage transport

One smaller but important ownership leak still remained after the earlier ACP event work:

- `ConversationTurnCoordinator` still effectively carried ACP-only runtime-event sink duplication
  and record-shape knowledge
- conversation persistence was drifting toward knowing too much about ACP runtime-event payload
  structure instead of only how to write conversation-lane records

That is now tightened further:

- `acp::BufferedAcpTurnEventSink` and `acp::CompositeAcpTurnEventSink` are the shared ACP-owned
  sink utilities used by both the ACP manager and conversation turn-coordinator path
- `acp::analytics::build_persisted_runtime_event_records(...)` owns the persisted
  `acp_turn_event` / `acp_turn_final` record shape
- `conversation::persistence::persist_acp_runtime_events(...)` now acts only as storage transport
  into the existing conversation event lane
- `conversation::ConversationTurnCoordinator` no longer keeps private ACP event sink or record
  builder duplication; it only decides whether a turn enters ACP and then wires the already-owned
  ACP pieces together

This matters because it keeps the seam honest:

- `conversation::ContextEngine` remains only the normal provider/context lifecycle seam
- `conversation::*` remains the turn orchestration and storage transport layer
- `acp::*` owns ACP runtime-event semantics, sink composition, provenance, and analytics shape
- `acpx` remains just one ACP runtime backend, not the place where global persistence semantics are
  invented

That is the safer pre-embed shape for future OpenClaw-style ACP bridge/runtime expansion as well:
live streaming, richer receipts, or alternative ACP backends can grow behind ACP-owned event
abstractions without forcing another coordinator-local rewrite.

### 23. Coordinator caller surfaces now default to options-first ACP extension

There was still one API-shape risk left even after moving turn provenance and runtime-event shape
ownership into `acp::*`:

- `ConversationTurnCoordinator` still exposed multiple public `*acp_event_sink(...)` variants
- callers could still keep treating ACP extensibility as "the event sink special case" instead of
  going through the full ACP-owned per-turn options seam

That shape tends to rot. The next ACP-local field would pressure the codebase into adding yet
another near-duplicate coordinator method instead of reusing the one extensibility slot that
already exists.

LoongClaw now tightens this further:

- `acp::AcpConversationTurnOptions` exposes small ACP-owned builder helpers for:
  - automatic vs explicit routing intent
  - event sink attachment
  - additive bootstrap MCP selection
  - working-directory hints
  - turn provenance
- `ConversationTurnCoordinator` now has options-first wrappers for both:
  - `session_id`
  - `session_id + runtime`
- legacy `*acp_event_sink(...)` entrypoints remain only as compatibility shims that normalize into
  `AcpConversationTurnOptions`
- real callers such as CLI chat and channel ingress now assemble ACP turn behavior through the
  ACP-owned options object directly rather than hand-populating ad hoc field sets
- ACP now also owns the first reusable operator-facing live stream sink:
  `acp::JsonlAcpTurnEventSink`, which lets chat print JSON-line ACP runtime events without keeping
  a chat-local printer type or inventing a second sink contract later

This matters because it keeps future ACP growth on the right rail:

- new ACP-only per-turn hints belong in `acp::AcpConversationTurnOptions`
- coordinator public API no longer needs one more entrypoint per ACP feature
- default provider/context-engine callers stay unchanged
- ACP still activates only after explicit request or route dispatch; options-first does not make it
  always-on

That is the cleaner long-term shape if LoongClaw later adds richer OpenClaw-style ACP receipts,
bridge-local knobs, or channel/workflow runtime hints without wanting another caller-surface
refactor.

### 24. ACP turn-entry policy now lives in `acp::runtime`, not in the coordinator

One more ownership leak remained even after moving caller surfaces to options-first APIs:

- `ConversationTurnCoordinator` still directly decided the ACP entry policy for each turn
- explicit ACP rejection when `acp.enabled=false`
- explicit ACP bypass of automatic dispatch gates
- automatic dispatch fallback back to the provider/context-engine path

Those are ACP route-policy concerns, not generic conversation concerns.

LoongClaw now centralizes that decision in `acp::runtime` through an ACP-owned turn-entry
evaluation seam. The normalized result is:

- `route_via_acp`
- `stay_on_provider`
- `reject_explicit_when_disabled`

`ConversationTurnCoordinator` now only consumes that ACP decision:

- if ACP says the turn is rejected, coordinator formats the error according to the existing
  inline/propagate mode
- if ACP says route via ACP, coordinator executes the ACP path
- if ACP says stay on provider, coordinator continues the normal provider/context-engine path

This matters because it tightens the ownership line another step:

- `conversation::*` owns orchestration and execution of the chosen path
- `acp::*` owns ACP entry policy, route policy, provenance, and runtime/control-plane semantics

That is closer to the OpenClaw shape as well. ACP policy can now evolve behind ACP-owned
abstractions without re-expanding coordinator-local ACP conditionals.

### 25. ACP route execution planning now lives in `acp::*`, not as coordinator-local assembly

Even after moving turn-entry policy into `acp::runtime`, one more ACP-heavy orchestration pocket
still remained in `ConversationTurnCoordinator`:

- prepare ACP route/bootstrap/request
- resolve ACP backend selection
- compose external and persistence event sinks
- invoke the shared ACP manager
- merge streamed events with final result events

That is all ACP route execution planning. It is not generic conversation orchestration.

LoongClaw now moves that orchestration behind an ACP-owned execution helper in `acp::runtime`.
The conversation layer now consumes a normalized ACP execution outcome:

- prepared route/request context
- selected backend identity
- succeeded vs failed turn result
- normalized runtime-event payloads ready for optional persistence

This matters because it shrinks the ACP-specific knowledge left inside
`ConversationTurnCoordinator`:

- `conversation::*` still owns whether/how to persist user and assistant turns
- `conversation::*` still owns inline vs propagate error presentation
- `acp::*` now owns turn-entry policy plus ACP route execution planning and event normalization

That is a safer pre-embed boundary for future OpenClaw-style ACP bridge/runtime growth. Streaming,
receipts, richer runtime details, or alternate ACP backends can evolve behind ACP-owned execution
helpers instead of forcing another coordinator-local rewrite.

### 26. ACP runtime-event persistence context is now ACP-owned

One last small but meaningful leakage remained after introducing the ACP execution helper:

- `conversation::*` still had to unpack ACP internals such as backend id, agent id, session key,
  binding scope, and request metadata just to persist ACP runtime events

That was still too much ACP shape knowledge in the conversation layer.

LoongClaw now makes the persistence context itself ACP-owned:

- `acp::PersistedAcpRuntimeEventContext` holds the normalized metadata needed to materialize
  `acp_turn_event` / `acp_turn_final` records
- ACP execution now returns that context alongside the normalized turn outcome
- conversation persistence only transports that context into the existing conversation-event lane

This tightens the boundary again:

- `acp::*` owns runtime-event record shape and the metadata context required to build it
- `conversation::*` owns only storage transport and user/assistant turn persistence policy

That is close to the shape we want for a stable pre-embed seam: conversation no longer needs to
know how ACP runtime metadata is assembled in order to persist ACP event history.

## What is still intentionally not done

These items are still outstanding and should remain separate workstreams:

1. Stream-oriented ACPX runtime execution parity with OpenClaw:
   - extend the new ACP-owned live stream seam beyond the reusable JSONL sink into richer
     daemon/channel default runtime surfaces; daemon chat now consumes the public sink directly,
     but most channel / operator-facing flows still consume persisted summaries instead of live
     streams or a transport-neutral subscription API
   - upgrade the current per-process abort path into richer process-group / supervisor semantics
   - richer status/details projection instead of the current normalized snapshot
2. Broader channel / CLI wiring for ACP bootstrap `mcp_servers` selection beyond the new
   dispatch-level default surface; daemon chat now supports per-turn additive selection, but
   channel and other workflow callers still need the same seam
3. Rich runtime cache eviction and observability snapshots
4. ACP bridge surface equivalent to `openclaw acp`

## Recommended next implementation order

1. Add a public live event stream surface on top of the persisted runtime-event shape so
   daemon/channel/conversation callers can choose direct streaming instead of summary-only
   observation; the ACP-owned JSONL sink is now the minimal base seam, but richer transport/API
   surfaces are still separate work.
2. Expand user-facing ACP bootstrap surfaces beyond `acp.dispatch.bootstrap_mcp_servers`, so
   configured backend-local `mcp_servers` can be selected per workflow without bespoke callers.
3. Evolve the observability snapshot further toward richer runtime-cache and session-identity
   diagnostics on top of the new actor/control queue view.
4. Add ACP bridge mode parity without collapsing it into ACPX backend mode.

## Anti-patterns to reject in review

- Adding ACPX command/version/permission fields directly to top-level `[acp]`
- Reusing `ConversationContextEngine` as the ACP backend abstraction
- Modeling ACP execution as a generic `provider` call
- Reverting shared backend instances back to per-call instantiation
- Passing only `session_key` into backend turn execution
- Treating `conversation_id` as optional noise instead of future binding identity
