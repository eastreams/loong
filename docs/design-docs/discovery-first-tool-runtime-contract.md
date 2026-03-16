# Discovery-First Tool Runtime Contract

## Purpose

This document defines the durable runtime contract for discovery-first provider
tool routing in LoongClaw.

The goal is to keep the provider-visible tool surface minimal, make non-core
tool access explicit and auditable, and prevent future refactors from drifting
back toward broad static tool exposure.

## Core Contract

### Provider-core tools

Only these tools are provider-callable by schema:

- `tool.search`
- `tool.invoke`

No other built-in tool or external skill should be directly exposed to the
provider as a first-round function schema without an explicit architecture
change.

### Discoverable tools

All other runtime tools are discoverable, not directly provider-callable.

This includes built-in non-core tools such as:

- `file.read`
- `file.write`
- `shell.exec`
- `claw.import`
- managed external-skill lifecycle and invoke tools

Discoverable tools may execute only through `tool.invoke` after discovery.

## Parser Contract

Provider parsers may receive legacy or provider-specific tool names such as:

- `file_read`
- `file.read`
- `shell_exec`
- `shell.exec`

If a provider response names a discoverable tool directly, the parser/runtime
 bridge must rewrite that intent into:

- `tool.invoke`
- with the discoverable target carried in `args_json.tool_id`
- with a valid short-lived lease carried in `args_json.lease`

This rewrite is compatibility behavior, not broad direct-tool permission.
Provider output is still expected to converge onto the discovery-first runtime
surface.

If provider payloads omit `session_id` or `turn_id`, the runtime must scope
them to the active session/turn before execution so lease validation remains
bound to the real conversation context.

## Lease Contract

`tool.search` returns compact result cards, not full provider schemas.

Each result card may include:

- canonical `tool_id`
- short summary
- compact argument hint
- required fields
- tags / match rationale
- a short-lived invoke lease

The lease must be validated by `tool.invoke` before dispatch. At minimum the
lease is expected to bind to:

- discovered tool id
- issuing session
- issuing turn
- short expiry / TTL

Execution must fail closed if the lease is invalid, expired, mismatched, or
missing.

## Follow-up Provider Turn Contract

If a provider turn executes `tool.search`, the coordinator must treat that turn
as discovery-only and request a follow-up provider turn before finalizing,
provided turn-round budget still permits it.

The follow-up turn receives:

- the original visible conversation context
- the assistant preface from the search turn
- a `[tool_result]` follow-up payload containing the search results

This contract exists so the model can select one concrete discovered tool after
search instead of terminating on the search payload itself.

### Raw-output mode

Raw-output requests do not bypass the discovery-first follow-up contract.

If the first turn is `tool.search` and the user requested raw tool output, the
runtime must still request the follow-up provider turn. Raw mode only changes
how the final invoked tool output is returned after the second round.

## Telemetry Contract

Discovery-first runtime telemetry is emitted through conversation events rather
than a separate DB or a second heavyweight persistence path.

The canonical event family is:

- `discovery_first_search_round`
- `discovery_first_followup_requested`
- `discovery_first_followup_result`

These events are intentionally compact and are used to quantify:

- how many search rounds occurred
- how many extra provider follow-up turns were requested
- how many follow-up rounds resolved to `tool.invoke`
- how often raw-output mode preserved the follow-up path
- best-effort estimated token growth caused by the follow-up context

Session-history loaders and analytics projections should read these persisted
conversation events instead of re-deriving the behavior from ad-hoc text logs.

## Security And Governance Boundaries

- Provider schema size is a security boundary as well as a token-cost boundary.
- Provider visibility must not silently widen when optional tools or external
  skills are installed.
- Non-core tool execution remains governed by the existing kernel/tool plane.
- Discovery compatibility rewrites are allowed only to funnel provider behavior
  back into `tool.invoke`, not to re-authorize direct discoverable tool calls.
- Missing scope metadata must be repaired before lease validation and execution.

## Unsupported Patterns

The current contract intentionally does not support:

- per-turn dynamic reinjection of discovered full tool schemas into provider
  requests
- direct provider access to discoverable tools as first-class schema entries
- database-backed tool discovery as a requirement for correctness
- search-time widening of provider-visible tools beyond the core
  `tool.search` / `tool.invoke` pair

These may be reconsidered later, but only as explicit architecture changes that
preserve the discovery-first default and its governance guarantees.
