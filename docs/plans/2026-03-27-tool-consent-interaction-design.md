# Tool Consent Interaction Design

## Goal

Deepen LoongClaw's current tool approval UX into a stable interaction contract that:

- keeps LoongClaw-native semantics instead of reusing Codex's `Allow / Always allow / Cancel`
  wording
- preserves the current execution-side safety boundaries
- separates execution policy from presentation policy
- supports text fallback today
- is ready for future CLI buttons, TUI choice screens, Feishu cards, and other interactive
  channel surfaces without inventing a new approval protocol per surface

This design is intentionally pre-binding. It defines the contract and rollout shape first, so
future surface work can bind to it without re-litigating semantics.

## Current Repo Facts

The current branch already has a meaningful base layer:

- `crates/app/src/conversation/turn_engine.rs`
  - session consent mode is evaluated independently from governed tool approval
  - `full` skips only the session-consent pause layer
  - `full` does not bypass governed approval or shell/kind-specific hard limits
- `crates/app/src/conversation/turn_shared.rs`
  - approval prompts now have a shared text contract
  - stable action ids already exist: `yes / auto / full / esc`
  - localized labels and summaries are derived from system-side data, not free-form model prose
- `crates/app/src/chat.rs`
  - CLI already renders the shared approval contract as a dedicated choice screen
- `crates/app/src/conversation/turn_coordinator.rs`
  - user replies are intercepted before normal provider routing
  - session choices are persisted on the root session scope
- `crates/app/src/tools/catalog.rs`
  - several tool families have already been reclassified so `auto` stops on higher-risk actions
- `crates/app/src/config/tools.rs`
  - `tools.consent.default_mode` is available in config
  - current default remains `full` for compatibility
- `crates/app/src/channel/mod.rs`
  - channel outbound payloads are still content-only
  - there is no first-class interactive approval payload yet

That means the execution semantics are no longer the main missing piece. The main remaining gap is
the contract between approval state and operator-facing surfaces.

## Why The Earlier Wording Was Wrong

The rejected wording family:

- `Allow`
- `Allow for this session`
- `Always allow`
- `Cancel`

is a poor fit for LoongClaw for three reasons.

### 1. It conflates consent with policy grants

LoongClaw now has at least three distinct layers:

1. hard limits
2. governed approval
3. session tool consent

`Always allow` incorrectly implies a durable policy grant. That is not what `full` means.

### 2. It hides scope

The important operator decision is not "allow or deny" in the abstract. The important question is:

- only this call?
- this session in low-risk mode?
- this session in broad tool-consent bypass mode?
- skip this call?

The old phrasing compresses those into a vague permission ladder.

### 3. It maps badly to future channel buttons

Button text should communicate effect directly. `Allow` is semantically thin. `Run once` or
`Skip call` are better button labels because they remain meaningful even when detached from the
full paragraph context.

## Core Design Principle

Treat tool approval as a two-part contract:

1. execution contract
2. presentation contract

The execution contract decides whether LoongClaw may proceed.

The presentation contract decides how the operator sees and controls that decision.

The current branch has started this separation, but not finished it. `turn_shared.rs` already owns
the action ids and text rendering, while the CLI still reconstructs its TUI screen by parsing the
rendered text back into structure. Channels still only receive plain reply strings.

The next architectural step should not be "more prompt text". It should be "one first-class
approval presentation object that text, CLI, and cards all project from."

## The Three Decision Layers

The approval stack should be modeled explicitly as three independent layers.

### 1. Hard Limits

These are not negotiable through `yes`, `auto`, or `full`.

Examples:

- shell denylist or shell allowlist failure
- kernel-context-required operations when no governed path is available
- malformed callback payloads
- invalid or stale approval request ids
- missing configuration or visibility violations

Hard limits fail closed. They are not approval prompts.

### 2. Governed Approval

This is the layer for actions that should remain operator-approved even if the session is in
`full`.

Examples:

- policy mutation
- durable config mutation
- capability acquisition and installation
- topology-expanding operations such as spawning or delegate-style authority expansion
- high-trust app tools that change provider/runtime authority

This should remain the place where LoongClaw asks for "do you approve this governed action?".
`full` must not collapse this layer into silent execution.

### 3. Session Tool Consent

This is the convenience layer that reduces repeated friction for routine tool use inside the
current session lineage.

Its modes are:

- `prompt`
- `auto`
- `full`

This layer is where the operator says how much routine interruption they want in the current
conversation flow.

## Stable Operator Actions

The action ids should remain stable and system-owned:

- `yes`
- `auto`
- `full`
- `esc`

These are protocol identifiers, not user-facing labels.

That distinction matters:

- text surfaces can expose them directly
- GUI surfaces can hide them behind localized buttons
- callback payloads should carry the ids, never label text
- parser logic should match ids and aliases, never translated prose

## Canonical Evaluation Order

The target policy should evaluate a pending tool call in this order.

### 1. Hard-Fail Preconditions

Reject immediately if any of the following fail:

- visibility and routing checks
- required config presence
- shell allowlist and denylist checks
- kernel-context requirements
- channel callback authenticity checks
- malformed or stale structured action payloads

This stage does not emit a consent prompt.

### 2. Approval-Control Exemption

Allow approval-control tools to bypass normal session-consent gating:

- `approval_request_resolve`
- `approval_request_status`
- `approval_requests_list`

Without this exemption, LoongClaw can deadlock by asking for approval to resolve approval.

### 3. Compute Governance Profile

For the target tool, compute at least:

- `governance_scope`
- `risk_class`
- `approval_mode`
- `capability_action_class`
- `tool_execution_kind`

The current code already has most of this in `ToolGovernanceProfile` and
`CapabilityActionClass`. The strategy should lean on that taxonomy instead of relying only on
tool-name allowlists.

### 4. Determine Prompt Kind

A pending tool call should map to one of these prompt kinds:

- `none`
- `session_tool_consent`
- `governed_approval`
- `combined`

Meaning:

- `none`
  - execute immediately
- `session_tool_consent`
  - the call is inside policy boundaries but still requires operator friction control
- `governed_approval`
  - the call crosses a policy boundary regardless of session mode
- `combined`
  - the current call needs governed approval and the operator is also still in a prompting session

The current implementation can continue with the simpler merged path, but the future contract
should expose this distinction explicitly so surfaces can explain the pause truthfully.

### 5. Resolve Against Session Mode

Only after prompt kind is known should LoongClaw consider:

- config default mode
- root-session override
- any one-shot approval resolution for the current request

Recommended precedence:

1. one-shot resolution for this request
2. root-session stored mode
3. config default mode

### 6. Execute Or Persist Pending Request

If blocked:

- persist the approval request
- emit structured presentation data
- render surface-specific fallback

If allowed:

- execute
- record the effective reason for execution

The execution reason should be auditable:

- `exempt`
- `session_auto`
- `session_full`
- `approve_once`
- `grant_reused`
- `policy_disabled`

## Action Semantics

### `yes`

Meaning:

- execute only the current pending tool call
- do not change session mode
- do not write persistent config

Recommended user-facing meaning:

- `只跑这次`
- `Run once`

### `auto`

Meaning:

- persist session mode as `auto` on the root session scope
- later low-risk tools may proceed without another session-consent pause
- higher-risk tools still pause

Recommended user-facing meaning:

- `本会话自动`
- `Session auto`

The detail line should make the boundary explicit:

- low-risk routine tools continue automatically
- writes, shell execution, provider switching, acquisition, installation, topology expansion, and
  governed mutations still pause

### `full`

Meaning:

- persist session mode as `full` on the root session scope
- stop asking for session tool consent in this session lineage
- still respect governed approval and hard limits

Recommended user-facing meaning:

- `本会话直通`
- `Session passthrough`

`full` should not be described as `always allow`.

Its detail line must state the limit plainly:

- this only disables the session-consent pause layer
- it does not create a durable grant
- it does not bypass governed approval
- it does not bypass shell allowlists or other hard safety checks

### `esc`

Meaning:

- skip the current pending tool call
- do not change session mode
- do not modify policy

Recommended user-facing meaning:

- `跳过本次`
- `Skip call`

`Cancel` is too vague. `Skip call` is better because it tells the operator what actually happens.

## Prompt Kinds And Action Behavior

The action ids stay stable, but their effect should be interpreted relative to prompt kind.

### `session_tool_consent`

Actions:

- `yes`
  - run this call once
- `auto`
  - run this call once and set root-session mode to `auto`
- `full`
  - run this call once and set root-session mode to `full`
- `esc`
  - skip this call

### `governed_approval`

Actions:

- `yes`
  - approve this governed call once
- `auto`
  - approve this governed call once and also set root-session consent mode to `auto`
- `full`
  - approve this governed call once and also set root-session consent mode to `full`
- `esc`
  - deny or skip this governed call

The important rule is that `auto` and `full` do not widen governed authority. They only add a
session-consent side effect around the same one-shot governed approval.

### `combined`

This is the most important future prompt type because it avoids misleading copy.

Meaning:

- the current call needs governed approval
- the current session would also have paused for consent

Actions should remain the same four stable ids, but the UI should explain the two-layer outcome
plainly:

- `yes`
  - approve this call once without changing future session friction
- `auto`
  - approve this call once and reduce future low-risk pauses
- `full`
  - approve this call once and stop future session-consent pauses
- `esc`
  - skip or deny this call

This avoids a common operator misunderstanding:

- `full` does not mean "trust this class of governed action forever"
- it means "approve this one governed action now, and reduce later routine consent friction"

### `kernel_required`

For kernel-context-required failures or similar cases, the surface should usually not show
`auto/full` controls at all. This is not a consent decision. It is a capability or routing failure.

That path should render explanation and recovery guidance instead of a consent control set.

## Copy Rules

### What Must Stay Stable

The following must be system-generated and authoritative:

- action ids
- callback payload ids
- numeric aliases where supported
- session-mode effect mapping
- policy semantics
- hard-limit disclaimers

### What May Adapt By Locale Or Surface

The following can vary:

- screen title
- explanatory bridge sentence
- choice labels
- help/footer wording
- button layout

### Important Constraint

Do not let the LLM invent the control vocabulary.

The model may generate the natural-language preface or the operator-facing explanation paragraph,
but the action contract must remain system-authored. Otherwise the system ends up parsing safety
decisions from untrusted prose.

That means the right split is:

- model or runtime can contribute contextual explanation
- system owns the actionable controls and their semantics

## Recommended Text Fallback

When a surface has no button or card support, the text fallback should be short and explicit.

Chinese fallback:

```text
可直接回复：yes / auto / full / esc
yes=只跑这次，auto=本会话仅自动运行低风险工具，full=本会话不再询问 tool consent 但仍受硬限制，esc=跳过本次
```

English fallback:

```text
Reply with: yes / auto / full / esc
yes = run this call only, auto = keep low-risk tools flowing in this session, full = stop asking for tool consent in this session but still respect hard limits, esc = skip this call
```

The exact bridge sentence can adapt by locale. The action ids should not.

This is better than the original:

- `send 'esc' to cancel, send 'yes' to run allow once, send 'auto' to auto run, send 'full' to allow all`

because it avoids broken English, avoids the misleading `allow all`, and uses words that match the
actual execution semantics.

## Localization Strategy

The bridge language should adapt to the user and channel context, but the safety contract must not
depend on the LLM improvising the controls.

Recommended locale resolution order:

1. explicit channel locale metadata
2. explicit session/operator locale setting
3. recent user-input language detection
4. English fallback

Recommended content split:

- model or runtime may provide:
  - contextual preface
  - explanation paragraph
  - task-specific natural-language lead-in
- system must provide:
  - action row
  - action labels
  - effect summaries
  - callback payload ids
  - hard-limit disclaimers

This gives the user adaptive language without turning the approval parser into a prompt-engineering
problem.

## TUI Design Guidance

Codex-style TUI influence is useful at the interaction level, but LoongClaw should keep its own
semantics.

### CLI Screen Shape

Recommended screen sections:

1. title
2. pause reason
3. current request metadata
4. four explicit choices
5. footer with raw keyword fallback

### Recommended Choice Labels

Chinese:

- `只跑这次`
- `本会话自动`
- `本会话直通`
- `跳过本次`

English:

- `Run once`
- `Session auto`
- `Session passthrough`
- `Skip call`

### Recommended Detail Lines

`只跑这次 / Run once`

- execute only the current tool call

`本会话自动 / Session auto`

- low-risk routine tools continue without another pause
- writes, shell, switching, acquisition, and governed actions still pause

`本会话直通 / Session passthrough`

- stop asking for session tool consent in this session
- governed approvals and hard limits still apply

`跳过本次 / Skip call`

- do not execute this tool call

### Input Methods

CLI should support:

- arrow selection + enter
- numeric aliases `1 / 2 / 3 / 4`
- raw keyword entry `yes / auto / full / esc`

The TUI should not depend on free-form text parsing for its primary interaction path. The free-form
parser should remain a compatibility fallback.

## Consent Eligibility Kernel

The current implementation uses a narrow baseline:

- `auto` is allowed when risk is `low`
- and approval mode is `never`

That is a good fail-closed start. The fuller strategy should evolve to an explicit consent
eligibility kernel that combines:

- `CapabilityActionClass`
- `ToolRiskClass`
- `ToolGovernanceScope`
- execution kind
- optional tool-specific overrides

Recommended derived classification:

```text
ToolConsentEligibility
  exempt
  auto_eligible
  prompt_only
  governed
```

Suggested default mapping:

- `Discover`
  - `auto_eligible`
- `ExecuteExisting`
  - `auto_eligible` when `low`
  - `prompt_only` when `elevated`
  - `governed` when `high`
- `CapabilityFetch`
  - `prompt_only` by default
- `CapabilityInstall`
  - `governed`
- `CapabilityLoad`
  - `prompt_only` or `governed` depending on future trust model
- `RuntimeSwitch`
  - `governed`
- `TopologyExpand`
  - `governed`
- `PolicyMutation`
  - `governed`
- `SessionMutation`
  - `prompt_only` by default

This strategy is better than a raw tool-name list because it scales with the catalog model already
present in `crates/app/src/tools/catalog.rs`.

## Recommended Auto-Mode Matrix

The policy for `auto` should be explicit enough that operators can predict it.

### Auto-Eligible By Default

- tool discovery
- read-only introspection
- safe list and status operations
- bounded web search and fetch
- read-only message/session queries
- non-mutating helper tools with low-risk governance profile

### Prompt In `auto`

- file writes and edits
- shell execution
- provider or runtime switching
- session archive, cancel, recover, or similar lifecycle mutations
- capability fetch and load unless explicitly downgraded later
- message send across sessions
- elevated-risk execution even when not fully governed

### Always Governed

- config mutation
- approval-policy mutation
- external capability install
- delegate and topology expansion
- explicit policy mutation
- high-trust app actions with `PolicyDriven` approval mode

## Session Mode Strategy

Session mode should be modeled as a root-lineage operator preference, not as an attribute of one
particular child session.

### Storage Rule

- store on the lineage root session
- write `updated_by_session_id`
- persist `updated_at`

### Read Rule

- resolve the lineage root first
- read session mode there
- never let child sessions silently shadow the root unless LoongClaw later introduces explicit
  sub-scope consent

### Reset Rule

Recommended future controls:

- `prompt`
  - restore prompting for the current session lineage
- `auto`
  - set low-risk auto mode
- `full`
  - set passthrough mode

There should eventually be an operator-visible way to downgrade a session from `full` back to
`auto` or `prompt` without editing config.

## Queueing And Multi-Pending Policy

The current text flow can get away with "resolve the first pending request" only while the pending
queue is effectively single-item.

The real strategy should support multiple pending approvals.

### Rules

- every blocked call gets a stable `request_id`
- queue ordering should be deterministic
  - recommended: oldest pending first
- structured surfaces should resolve by explicit `request_id`
- bare text replies such as `yes` should only auto-bind when exactly one pending request exists

### When More Than One Pending Exists

The surface should expose:

- count of pending requests
- current queue position
- request-specific controls

If the surface cannot render request-specific controls, LoongClaw should ask for disambiguation
instead of guessing.

## Structured Action Payload Security

Future buttons and cards should use one-shot structured payloads.

Recommended payload fields:

- `request_id`
- `action_id`
- `session_scope_id`
- `turn_id`
- `nonce`
- `issued_at`
- optional `expires_at`

Recommended validation steps:

1. verify channel callback authenticity
2. parse structured payload
3. load request by `request_id`
4. confirm it is still pending
5. confirm session visibility and actor legitimacy
6. confirm nonce matches and is unused
7. confirm payload age is acceptable
8. resolve the action

Recommended expiry rule:

- short TTL for button payloads
- stale payload resolves to safe no-op or explicit stale-state response

Recommended replay rule:

- nonce should be one-shot
- duplicate callbacks should be idempotent and non-destructive

## Audit And Observability Strategy

Approval UX is part of the safety surface. It should therefore be observable.

Each approval lifecycle should record:

- `request_created`
- `request_prompt_rendered`
- `request_action_received`
- `request_resolved`
- `session_mode_changed`
- `config_mutation_requested`
- `config_mutation_approved`
- `replay_started`
- `replay_succeeded`
- `replay_failed`
- `callback_rejected_stale`
- `callback_rejected_replay`

Suggested audit dimensions:

- session id
- root session id
- request id
- tool name
- prompt kind
- action id
- locale
- channel or surface type
- execution reason
- failure code if any

This makes the approval layer debuggable without reading raw chat logs.

## Persistent Config Mutation Strategy

The config path should be explicit instead of being a hidden side effect of approval buttons.

Recommended future action model:

- approval prompt changes session behavior only
- persistent config changes happen through a dedicated tool or settings action

If LoongClaw later supports a settings tool, its contract should look more like:

- `set_tool_consent_default_mode(prompt|auto|full)`

than:

- "press full and silently rewrite config"

Recommended guardrails for persistent config mutation:

- always treated as governed
- show old value and new value before write
- show scope clearly
  - `this session`
  - `future sessions`
- require explicit confirmation even when current session is already in `full`
- persist an audit event with config diff summary

## Surface-Specific Rendering Strategy

Different surfaces should render the same contract differently.

### Plain Text Channels

- render title, reason, and fallback command line
- no structured callback dependency

### CLI TUI

- primary path is selection UI
- text command row remains visible as fallback
- numeric aliases are CLI-only convenience, not a cross-channel protocol requirement

### Card/Button Channels

- primary path is structured button payloads
- labels may be localized
- raw action ids may be hidden from the user
- stale interactions should update the card state or return safe toast feedback

### Streaming Surfaces

- do not stream partial approval controls
- finalize the approval presentation atomically

Approval controls should appear as one stable interaction checkpoint, not as token-stream noise.

## Future Presentation Contract

The next clean boundary should be a structured assistant reply presentation object. The exact type
name can vary, but the shape should look like this:

```text
AssistantReplyPresentation
  text: String
  locale: ReplyLocale
  approval_prompt: Option<ApprovalPromptPresentation>
  fallback_message: Option<String>
```

And:

```text
ApprovalPromptPresentation
  prompt_kind: tool_consent | governed_approval | kernel_required | combined
  request_id: String
  tool_name: Option<String>
  rule_id: Option<String>
  reason: String
  preface: Option<String>
  actions: Vec<ApprovalActionPresentation>
  pending_count: usize
  queue_position: usize
```

And:

```text
ApprovalActionPresentation
  action_id: yes | auto | full | esc
  effect: current_call_only | session_auto | session_full | skip_current_call
  label: String
  summary: String
  detail_lines: Vec<String>
  recommended: bool
  callback_payload: StructuredApprovalActionPayload
```

And:

```text
StructuredApprovalActionPayload
  request_id: String
  action_id: yes | auto | full | esc
  session_scope_id: String
  turn_id: String
  nonce: String
  expires_at: Option<Timestamp>
```

The important point is not the exact Rust type name. The important point is that:

- conversation code emits structure first
- text rendering becomes one projection
- CLI TUI becomes another projection
- Feishu cards become another projection
- button callbacks carry the same action ids and request ids

## Why Request Id Must Be Carried In Structured Inputs

The current text fallback can safely assume "the first pending request" only while the interaction
surface is simple and the pending queue is short.

That assumption does not scale to:

- multiple blocked tool calls
- background lanes
- cards reopened later
- callback retries
- multiple pending approvals across child sessions

Structured inputs must therefore carry `request_id`.

Text fallback may still allow a bare `yes` when there is exactly one pending request. If more than
one exists, the surface should switch to request-specific resolution instead of guessing.

## Channel Capability Model

The current `ChannelOutboundMessage` model is content-only. Future approval binding should add a
separate capability layer rather than overloading message text.

Recommended capability split:

```text
ChannelInteractionCapability
  text_fallback
  choices_inline
  button_actions
  card_actions
  callback_payloads
  delayed_update
```

Surface behavior should then follow capability detection:

- `text_fallback`
  - render the fallback lines only
- `choices_inline`
  - render labels with numeric aliases or slash-like quick actions
- `button_actions`
  - render localized buttons and keep text minimal
- `card_actions`
  - render richer reason/details and button groups
- `callback_payloads`
  - callbacks send structured ids instead of label text
- `delayed_update`
  - allow the approval card to update in place after resolution

This keeps the approval protocol stable while letting each channel choose the richest safe
presentation it supports.

## Feishu And Other Card Surfaces

For Feishu, Slack-style interactive cards, and similar channels, the key rule is:

button payloads must carry structured approval action payloads, not free-form natural language.

That means:

- button label can be localized
- button payload should contain `request_id` and `action_id`
- callback handler resolves only from structured ids
- the assistant text shown on the card is not the source of truth

This keeps future callback routing compatible with the existing `approval_request_resolve` replay
path.

## Session vs Persistent Config

The user explicitly wants two different scopes:

- session-only change
- config-backed default change

That distinction should remain explicit in both UX and security policy.

### Session-Only Change

Changing to `auto` or `full` from an approval prompt is a session mutation.

Properties:

- persists only on the root session scope
- survives later turns in the same lineage
- does not rewrite `config.toml`
- should be safe to perform from the approval control path itself

### Persistent Config Change

Changing `config.toml` should be treated as a governed config mutation, not as an incidental side
effect of pressing `auto` or `full`.

Properties:

- changes future conversations
- should be exposed through a dedicated settings/config mutation path
- should require governed approval by default
- should remain stoppable even when the current session is already in `auto`
- should also remain stoppable when the session is in `full`

This is the clean way to satisfy the product requirement:

- session choice affects this conversation
- config mutation affects future conversations
- the second one is a stronger act and should have stronger consent semantics

## Recommended Policy Matrix

### `prompt`

Behavior:

- every non-exempt tool call pauses for operator confirmation

Use cases:

- conservative workflows
- demos
- operator training
- debugging tool routing

### `auto`

Behavior:

- low-risk routine tools run automatically
- anything that crosses a stronger boundary pauses

Should be auto-eligible:

- tool discovery
- list and status queries
- read-only session inspection
- read-only message inspection
- bounded web search and fetch
- safe metadata lookups

Should pause in `auto`:

- file edits and writes
- shell execution
- provider switch
- runtime mutation
- session mutation beyond the current consent action
- delegate or topology expansion
- external skill fetch, install, load, invoke, or remove
- durable config mutation
- anything already classified as governed high-trust app execution

### `full`

Behavior:

- session-consent pauses are skipped
- governed approval and hard limits still apply

This is the highest-friction-reduction mode, not the highest-authority mode.

That distinction is critical.

`full` is acceptable when the operator wants LoongClaw to keep moving inside the existing legal
execution envelope. It must not silently widen that envelope.

## Why Current Default Should Not Be Flipped Blindly

The current code keeps `tools.consent.default_mode = full`.

That is not the ideal long-term beginner default, but changing it immediately would change current
behavior broadly and already caused regressions during this work.

Recommended product direction:

- keep the config default stable for now
- let onboarding, autonomy profile, or future product presets suggest better starting modes

Possible future mapping:

- `discovery_only` -> recommend `prompt`
- `guided_acquisition` -> recommend `auto`
- `bounded_autonomous` -> recommend `full`

This can be additive without silently breaking existing users.

## Security Invariants

These invariants should remain non-negotiable.

### 1. Never Parse Security Decisions From Model Prose

Natural-language explanation is display-only.

Action resolution must come from:

- stable text ids and aliases
- numeric aliases
- structured callback payloads

### 2. Never Let `full` Mean Policy Mutation Grant

`full` changes session friction, not authority.

### 3. Always Resolve Structured Buttons Against Live Pending State

On callback:

- load the request by `request_id`
- confirm it is still pending
- confirm session visibility rules
- reject stale or replayed payloads safely

### 4. Prefer Root-Session Scoping For Session Consent

Session consent should continue to live at the lineage root, so child sessions do not silently
fork operator intent.

### 5. Make Config Mutation Explicit

Do not hide `config.toml` rewrites behind `auto` or `full`.

## Rollout Plan

### Phase 1

Keep the current text contract and CLI screen, but document the semantics clearly.

This document is that phase.

### Phase 2

Introduce a first-class `AssistantReplyPresentation` or equivalent structure in conversation code.

Text reply remains a projection, not the source of truth.

### Phase 3

Make CLI consume the structured presentation directly instead of reparsing rendered text.

### Phase 4

Add channel interaction capability metadata and a generic approval-action projection path.

### Phase 5

Bind the first real card/button surface, likely Feishu callback cards, using structured action
payloads with `request_id`.

### Phase 6

Retain plain-text reply parsing only as a backward-compatible fallback path for text-only surfaces.

## Decision Summary

The right LoongClaw direction is not:

- `Allow`
- `Allow for this session`
- `Always allow`
- `Cancel`

The right direction is:

- stable protocol ids: `yes / auto / full / esc`
- LoongClaw-native labels that describe effect instead of vague permission
- explicit separation of session consent, governed approval, and hard limits
- structured presentation metadata for future buttons and cards
- session changes and persistent config changes treated as different scopes with different safety
  expectations

That gives LoongClaw a consent model that is:

- clearer for CLI users
- safer for future interactive surfaces
- more truthful about what `auto` and `full` really do
- compatible with the current implementation work already in progress
