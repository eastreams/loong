# Feishu Card Delay Update Design

## Goal

Add a controlled Feishu delayed card update capability on top of the existing callback transport so that:

- Feishu callback tokens can be used safely after the webhook returns `HTTP 200`
- LoongClaw can update the card through the official delayed update API
- the callback token remains private to tool execution and is not exposed in model-visible ingress notes
- the existing layering stays intact:
  - `channel/feishu` only carries webhook and callback transport concerns
  - `app/src/feishu/*` owns the delayed card update client/resource logic
  - `tools/feishu` exposes a narrowly scoped card update adapter only when Feishu runtime is configured

## Official Constraints

Primary source references from Feishu Open Platform:

- Handle card callbacks
  - https://open.feishu.cn/document/uAjLw4CM/ukzMukzMukzM/feishu-cards/handle-card-callbacks
- Card callback communication
  - https://open.feishu.cn/document/feishu-cards/card-callback-communication
- Delay update message card
  - https://open.feishu.cn/document/server-docs/im-v1/message-card/delay-update-message-card

Relevant constraints from the docs:

- Callback requests must receive `HTTP 200` within 3 seconds.
- For delayed update flows, the callback response body can be `{}`
  - or a `toast` object.
- The delayed update must happen after the callback response is sent.
- The callback `event.token` is the credential for delayed updates.
- The delayed update token is valid for 30 minutes and can only be used twice.
- Delayed updates use `POST /open-apis/interactive/v1/card/update`.
- The API uses `tenant_access_token`.
- `open_ids` are required for non-shared cards (`config.update_multi=false`) and must not be used for shared cards (`update_multi=true`).

## Current State

Current Feishu support now includes:

- parsing `card.action.trigger` and legacy callback payloads
- safe `{}`
  callback responses
- provider reasoning over normalized callback summaries

Current gaps:

- the callback token is parsed but unused
- there is no Feishu client/resource helper for delayed card updates
- there is no Feishu tool that can update a card after callback processing
- there is no private callback tool context for Feishu tools

## Option Analysis

### Option A: Expose callback token directly in the model-visible summary

Pros:

- smallest implementation
- no extra ingress plumbing

Cons:

- leaks a time-bound write credential into model-visible context
- makes token handling harder to reason about
- weakens the security posture for tool execution

### Option B: Generic cross-channel callback secret store

Pros:

- future-proof if many channels need callback secrets

Cons:

- too much abstraction for the current scope
- risks pushing Feishu-specific behavior into generic conversation state
- adds complexity before a second channel actually needs it

### Option C: Recommended Feishu-private tool context + delayed update tool

Pros:

- preserves current layering
- keeps callback token out of model-visible prompt state
- enables controlled delayed card updates through a Feishu-only tool
- scales to future toast/card response work without redesigning the transport again

Cons:

- needs one extra private ingress path for tool defaults
- requires a new Feishu resource client helper and tool schema

Recommendation: Option C.

## Chosen Architecture

### 1. Add private Feishu callback tool context

Extend channel delivery / ingress plumbing so callback token and callback-scoped defaults can reach Feishu tools without appearing in `ConversationIngressContext::system_note()`.

The private context should include:

- callback token
- callback open_message_id
- callback open_chat_id
- operator open_id as the default exclusive-card update target

### 2. Add Feishu delayed card update resource client

Create a Feishu resource helper under `app/src/feishu/resources/*` that calls:

- `POST /open-apis/interactive/v1/card/update`

It should validate:

- token is non-empty
- card is an object
- `open_ids` are normalized and deduplicated when supplied

### 3. Add a Feishu-only card update tool

Add a new tool under the Feishu tool surface, for example:

- `feishu.card.update`

MVP payload should support:

- `account_id`
- `open_id`
- `callback_token`
- `card`
- `open_ids`

Tool defaults:

- default `account_id` from current Feishu ingress
- default `callback_token` from private callback tool context
- default `open_ids` to `[operator_open_id]` only when callback context exists and the caller omitted `open_ids`

The tool should not attempt to infer whether the card is shared or exclusive. If the caller needs shared-card semantics, they can omit `open_ids`; the Feishu API will enforce correctness.

### 4. Keep transport behavior conservative

Do not add generic model-driven immediate callback card responses in this phase.

The webhook should continue to:

- return `{}`
  by default
- run provider logic safely
- allow the assistant to invoke `feishu.card.update` inside the callback turn when it has enough information to do so

### 5. Observability and docs

Expose the new capability in:

- Feishu tool registry / provider schema
- callback quality notes
- optionally channel notes if needed later

## Data Flow

1. User clicks a card button.
2. Feishu sends `card.action.trigger`.
3. `channel/feishu` validates and normalizes the callback.
4. The callback token is stored in private Feishu tool context on the inbound message.
5. Conversation reasoning sees only the non-sensitive callback summary.
6. If the turn chooses `feishu.card.update`, the tool adapter receives the private callback token through `_loongclaw`.
7. `app/src/feishu/resources/*` calls the delayed update API with tenant auth.

## Non-Goals

- generic cross-channel callback secret handling
- automatic inference of exclusive vs shared card update policy beyond safe defaults
- arbitrary immediate callback card JSON generation from generic assistant prose
- background retry orchestration for expired or exhausted callback tokens

## Expected Outcome

After this phase, LoongClaw will support a secure delayed-update loop for Feishu cards:

- webhook callback arrives
- callback token is retained privately
- the assistant can update the originating card via a Feishu-specific tool
- the token never needs to be exposed in prompt-visible context
