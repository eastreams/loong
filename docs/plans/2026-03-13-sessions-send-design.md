# Sessions Send Design

## Context

LoongClaw now has a fairly thick session and delegate control surface for:

- discovery via `sessions_list`
- transcript and event inspection via `sessions_history` and `session_events`
- status and bounded waiting via `session_status` and `session_wait`
- remediation via `session_cancel` and `session_recover`
- child creation via `delegate` and `delegate_async`

That still leaves one major operator gap compared with the external references:

- OpenClaw's minimum thick control surface includes `send` or `steer`
- NanoBot emphasizes `message`
- the original LoongClaw phase-1 design explicitly deferred cross-channel `sessions_send`

## Problem

We need to add one real outbound control primitive without pretending the current runtime already
has capabilities it does not have.

The critical architectural constraint is that LoongClaw does **not** currently have:

- a live inbox for running delegate children
- safe real-time steering of in-flight child turns
- safe restart or resume semantics for terminal sessions

That means a direct `session_send` or `session_steer` tool for running delegate children would be
misleading. It would expose a name without a truthful execution model.

## Goals

- Add a real outbound operator tool that can send a text message to a known external session.
- Keep the capability anchored to stable session ids instead of raw provider-specific target ids.
- Avoid mutating transcript history or pretending to execute a new turn in the target session.
- Keep child delegate sessions unable to send outbound messages.
- Require explicit opt-in through config.

## Non-Goals

- No live steering of running delegate children.
- No generic outbound messaging to arbitrary destinations.
- No implicit transcript append to the target session.
- No broadening of session visibility for delegate children.
- No background queue or inbox for later target-session execution.

## Options Considered

### Option 1: Direct `session_send` into visible delegate children

Pros:

- closest to OpenClaw's `send` or `steer`
- keeps everything inside the session/delegate model

Cons:

- current runtime has no truthful live inbox for running children
- queued async children already have their task bound at spawn time
- terminal children have no safe restart semantics

Rejected.

### Option 2: Generic `message_send(channel, target, text)`

Pros:

- straightforward adapter-level implementation
- does not depend on the session registry

Cons:

- bypasses the session model entirely
- weakens audit readability by using raw channel-specific target ids
- broadens authority to arbitrary configured destinations

Rejected for this slice.

### Option 3: Constrained `sessions_send(session_id, text)` for known channel-backed root sessions

Pros:

- uses the stable session id model already visible to operators
- does not fake subagent steering semantics
- can be restricted to previously seen root sessions only
- composes naturally with the existing session registry and audit surface

Cons:

- does not solve delegate-child steering yet
- requires async app-tool dispatch with channel config access

Chosen.

## Chosen Approach

Add a new app tool: `sessions_send`.

### Request model

`sessions_send` accepts:

- `session_id: string`
- `text: string`

The target `session_id` must resolve to a known root session backed by a currently supported
outbound channel:

- `telegram:<chat_id>`
- `feishu:<chat_id>`

### Safety boundary

The tool only works when all of the following are true:

- `tools.messages.enabled = true`
- the target session already exists in session metadata or legacy transcript fallback
- the target session is a root session, not a delegate child
- the target session uses a supported channel-backed session id prefix
- the corresponding channel is enabled in config
- the target id is still present in the configured channel allowlist

If any of those fail, the tool returns a hard error rather than silently broadening authority.

### Visibility model

`sessions_send` is exposed only to root sessions.

Delegate child tool views continue to exclude outbound messaging. This preserves the earlier
guardrail that child sessions cannot send external messages.

### Delivery semantics

`sessions_send` sends a text message through the relevant channel adapter:

- Telegram: bot sendMessage path
- Feishu: text message path through the existing adapter and tenant-token refresh flow

It does **not**:

- append a user turn to the target transcript
- execute a provider turn for the target session
- change target session state

### Evidence and observability

On success, `sessions_send`:

- returns a structured delivery receipt payload
- appends a non-transcript control event to the target session

The event should record only operational metadata such as:

- channel kind
- target id
- actor session id
- text length

It should not persist the full outbound message text in session events.

## Testing

Required coverage:

1. config defaults keep outbound messaging disabled
2. config TOML round-trips the new `tools.messages.enabled` switch
3. provider schema exposes `sessions_send` only when enabled
4. root sessions can send to a known Telegram or Feishu session through an injected fake sender
5. unknown sessions, delegate-child targets, and child callers are rejected
6. successful send appends a control event but does not append transcript rows

