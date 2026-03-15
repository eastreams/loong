# Feishu Card Callback Design

## Goal

Add first-class Feishu card callback support to LoongClaw's Feishu channel so that:

- the Feishu webhook endpoint accepts card callback events in addition to message receive events
- callback requests are security-verified and normalized into LoongClaw ingress context
- the transport can return a Feishu-compliant callback response within the 3-second requirement
- the design stays layered:
  - `channel/feishu` owns webhook parsing, callback payload normalization, and callback HTTP responses
  - `app/src/feishu/*` remains the home for Feishu API/resource/client logic
  - generic conversation code remains channel-agnostic

## Reference Constraints

Primary source references from Feishu Open Platform:

- New card callback structure: `card.action.trigger`
  - https://open.feishu.cn/document/feishu-cards/card-callback-communication
- Card callback handling guide
  - https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/handle-card-callbacks
- Earlier callback structure: `card.action.trigger_v1`
  - https://open.feishu.cn/document/deprecated-guide/message-card/configuring-card-callbacks/card-callback-structure

Relevant constraints extracted from the Feishu docs:

- Webhook callback requests must return `HTTP 200` within 3 seconds.
- The new callback event type is `card.action.trigger`.
- The older callback type `card.action.trigger_v1` may still be delivered and may coexist with the new type.
- Valid callback bodies may be:
  - `{}`
  - `toast`
  - `toast + card`
- In the new callback structure, the verification token is inside `header.token`, not the top-level `token`.
- In the new callback structure, card-update credentials are carried in `event.token`.

## Current State

Today the Feishu webhook parser only accepts:

- `url_verification`
- `im.message.receive_v1`

Everything else is ignored.

That means LoongClaw can:

- receive inbound Feishu messages
- summarize interactive card messages as content
- reply back to Feishu messages

But it cannot:

- receive actual card action callbacks when a user clicks a card button
- preserve callback-specific context such as callback token, action value, form value, operator identity, and host context
- return Feishu callback-native responses such as `toast` or card updates

## Option Analysis

### Option A: Treat card callbacks as plain inbound text only

Interpret callback events as structured text, feed them into the existing conversation flow, and always return `{}` to Feishu.

Pros:

- smallest code change
- lowest risk to existing abstractions

Cons:

- no callback-specific response model
- no clean path for immediate `toast` or card updates
- transport semantics would remain under-modeled

### Option B: Full callback DSL at the conversation layer

Let the model or tools directly emit Feishu callback response objects, including arbitrary raw/template card updates.

Pros:

- highest expressiveness
- immediate card interaction support

Cons:

- pollutes generic conversation abstractions with Feishu-specific response contracts
- increases validation and safety burden sharply
- likely to be brittle for MVP

### Option C: Recommended hybrid transport model

Introduce a Feishu callback event and callback response model in `channel/feishu`, while keeping conversation transport-agnostic.

Pros:

- preserves layering
- supports Feishu-compliant immediate responses
- allows callback data to flow into conversation via normalized ingress
- leaves room for later controlled card-update features

Cons:

- requires a new transport branch and a small Feishu-only callback response adapter

Recommendation: Option C.

## Chosen Architecture

### 1. Add Feishu callback event parsing in `channel/feishu`

Extend the webhook parser to detect:

- `header.event_type == "card.action.trigger"`
- legacy callback payloads that match the earlier `card.action.trigger_v1` shape

Both variants will normalize into one internal callback event model.

### 2. Introduce a Feishu callback event model

Add a new event structure alongside `FeishuInboundEvent`:

- callback event id
- callback version (`v2` or `v1`)
- operator identity
- callback token for delayed/immediate card update
- action payload
  - tag
  - name
  - value
  - form_value
  - timezone
- host/context payload
  - open_message_id
  - open_chat_id
  - preview/link context when present

This model stays inside `channel/feishu`.

### 3. Normalize callback events into conversation ingress

The callback event will produce:

- a `ChannelSession` keyed by Feishu account + chat + optional message/thread context
- a text summary describing the callback action in a deterministic, high-signal format
- ingress metadata containing message ids, operator identity, and related message context

This lets the existing turn engine reason over callback events without making the conversation layer know about Feishu callback transport contracts.

### 4. Add a Feishu callback response model in transport only

Add a Feishu-only response enum, owned by `channel/feishu`, with three safe modes:

- `Noop`
  - serializes to `{}`
- `Toast`
  - serializes to `{ "toast": ... }`
- `Card`
  - serializes to `{ "toast": ..., "card": ... }` or `{ "card": ... }`

This model is not exposed as a generic channel abstraction.

### 5. MVP response behavior

For this phase, the default callback handling path should be:

- always return a valid `HTTP 200`
- by default return `{}` when no immediate callback response is available
- optionally return a deterministic success/error toast in transport-managed situations

The first implementation should not require generic LLM output to synthesize arbitrary card JSON.

That keeps the 3-second contract reliable and avoids coupling generic assistant prose to Feishu card schemas.

### 6. Backward compatibility

The message receive path remains unchanged.

The callback path will be additive:

- existing `im.message.receive_v1` handling stays intact
- callback support is enabled on the same webhook endpoint
- old and new callback request structures can coexist without breaking message ingress

## Data Flow

1. Feishu sends webhook POST to existing Feishu endpoint.
2. Webhook verifies signature and decrypts payload if needed.
3. Parser branches to one of:
   - url verification
   - message receive
   - card callback
   - ignore
4. For card callback:
   - normalize to internal callback event
   - reserve dedupe key
   - build channel/conversation ingress context
   - run provider/conversation handling
   - map callback transport result to Feishu-compliant JSON body
   - return `HTTP 200` within contract

## Error Handling

The transport should distinguish:

- auth/signature/token failures
  - keep current `401`/`400` behavior
- duplicate callback events
  - return success-safe no-op body
- provider/runtime failures
  - prefer Feishu-safe callback failure handling over malformed bodies
- unsupported callback shapes
  - ignore safely if clearly non-target
  - reject only when the payload claims to be a callback but is malformed

## Testing Strategy

### Parser tests

- parses new callback payload
- parses old callback payload
- accepts token from `header.token` for new callback payloads
- preserves operator/action/context fields
- ignores unrelated event types

### Webhook tests

- callback webhook returns `HTTP 200` and valid callback body
- duplicate callback is deduped safely
- provider failures do not produce malformed Feishu callback responses

### Regression tests

- existing message receive webhook tests remain green
- url verification remains green
- signature verification semantics stay unchanged except for callback token extraction logic

## Non-Goals for This Phase

- full generic LLM-driven card JSON generation
- a universal cross-channel callback abstraction
- automatic delayed card update orchestration via Feishu update APIs
- adding new Feishu tools unless callback transport implementation clearly requires a narrowly scoped helper later

## Expected Outcome

After this phase, LoongClaw will be able to accept Feishu interactive card callback events on the existing Feishu webhook endpoint, preserve the action context for reasoning, and respond to Feishu with compliant callback bodies without breaking the current message webhook flow.
