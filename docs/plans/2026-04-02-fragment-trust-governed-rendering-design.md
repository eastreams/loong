# Fragment Trust And Governed Prompt Rendering Design

Date: 2026-04-02
Issue: #814
Predecessor: #758, PR #771
Status: approved for implementation

## Goal

Move prompt trust policy from scattered source-local sanitizers into the prompt
orchestration layer so advisory or untrusted context cannot regain system-prompt
authority by accident as new prompt fragment sources are added.

## Current State

LoongClaw now assembles the provider-facing system prompt from typed
`PromptFragment` values.

That is a strong structural improvement, but the trust boundary is still split
across multiple call sites:

- `ToolDiscoveryState::render_delta_prompt(...)` sanitizes advisory values with
  local helpers before rendering `[tool_discovery_delta]`
- advisory memory messages are sanitized separately through
  `advisory_prompt::demote_governed_advisory_headings_with_allowed_roots(...)`
- `PromptCompiler` currently sees only lane ordering and dedupe; it has no
  explicit notion of trusted versus advisory fragment rendering

This means the architecture is typed, but the rendering policy is still
distributed.

If a future advisory fragment source forgets to add its own heading demotion,
line flattening, or prompt-shaped text neutralization, it can reopen prompt
injection or section-spoofing risks.

## Non-Goals

- do not redesign provider message shapes
- do not introduce multi-system-message provider behavior
- do not weaken hidden-tool discovery or `tool.invoke` lease validation
- do not implement the larger pre-assembly memory pipeline in this slice
- do not turn all advisory memory into prompt fragments in one pass

## Options Considered

### Option 1: keep source-local sanitization and add more tests

This is the smallest diff in the short term, but it keeps the real problem in
place.

The trust boundary would still be “every source remembers to sanitize itself.”

This is rejected because it does not create a durable architectural guarantee.

### Option 2: add fragment trust policy plus a shared governed renderer

Add explicit rendering policy to `PromptFragment`.

Teach `PromptCompiler` to apply governed rendering for advisory fragments.

Move advisory line-demotion and inline advisory value rendering into one shared
module owned by prompt orchestration.

Keep source-local structure where needed, but remove source-local ownership of
the trust rules.

This is the recommended option because it fixes the architectural gap without
forcing a larger memory-runtime refactor.

### Option 3: convert all advisory runtime context into prompt fragments first

This would create the cleanest long-term model, but it broadens the change
surface into memory assembly and provider runtime behavior.

This is rejected for the first slice because it is larger than the root cause.

## Chosen Design

Use option 2.

Introduce one prompt-orchestration-owned rendering policy and one shared
governed advisory renderer.

### 1. Add explicit fragment render policy

Extend `PromptFragment` with a small rendering policy enum.

The initial shape should stay narrow. A likely starting point is:

- `TrustedLiteral`
- `GovernedAdvisory { allowed_root_headings: &'static [&'static str] }`

`TrustedLiteral` means compiler output is rendered verbatim after existing trim
and dedupe behavior.

`GovernedAdvisory` means the compiler must demote governed headings and preserve
only explicitly allowed top-level advisory container headings.

This makes trust visible in the fragment model instead of implicit in the call
site that produced the fragment.

### 2. Centralize governed advisory rendering

Create a shared prompt-rendering helper module inside the conversation layer.

That module should own two distinct concerns:

- whole-fragment governed rendering for advisory blocks
- inline advisory value rendering for untrusted data interpolated into
  structured advisory prose

The existing `advisory_prompt` logic is a strong starting point and should be
reused or moved rather than duplicated.

The important design rule is ownership:

- trust policy lives with prompt orchestration
- sources may define structure
- sources do not define their own sanitization semantics anymore

### 3. Keep `tool_discovery_delta` structured but remove bespoke trust logic

`tool_discovery_delta` still needs structured output because it mixes headings,
refresh examples, hints, and required field groups.

The minimal correct move is:

- keep `ToolDiscoveryState` responsible for assembling the logical sections
- move advisory inline-value sanitization onto the shared governed renderer
- mark the resulting fragment with advisory render policy so compiler-level
  heading demotion still applies consistently

This keeps the existing feature shape while moving the trust contract into one
shared place.

### 4. Move advisory memory onto the same governed renderer

Advisory memory messages do not need to become fragments in this slice.

They do need to stop owning a separate heading-demotion implementation path.

`append_advisory_memory_message(...)` should reuse the same governed advisory
renderer module that prompt fragments use.

That gives LoongClaw one prompt-trust vocabulary across:

- advisory prompt fragments
- advisory memory messages
- future advisory surfaces

### 5. Preserve current runtime contracts

This change must preserve:

- single system message provider contract
- current prompt lane ordering
- current artifact mapping behavior
- hidden-tool visibility boundary
- fresh-lease requirement for `tool.invoke`

The work is about rendering trust, not capability semantics.

## Scope Of Code Changes

### Prompt orchestration model

- `crates/app/src/conversation/prompt_fragments.rs`
- `crates/app/src/conversation/prompt_orchestrator.rs`
- `crates/app/src/conversation/mod.rs`

### Shared governed renderer

- likely new conversation-local module for governed prompt rendering
- reconcile with `crates/app/src/advisory_prompt.rs`

### Advisory fragment sources

- `crates/app/src/conversation/tool_discovery_state.rs`
- `crates/app/src/provider/request_message_runtime.rs`

### Regression coverage

- prompt compiler tests
- `ToolDiscoveryState` rendering tests
- advisory memory sanitization tests
- end-to-end conversation runtime tests proving prompt-shaped advisory input
  cannot create fake system sections or fake tool instructions

## Why This Is Minimal And Correct

The root cause is not that advisory context exists.

The root cause is that trust policy is not a first-class part of prompt
compilation.

This design fixes that root cause at the architectural seam where fragments are
compiled and advisory text becomes prompt-visible content.

It avoids hardcoded keyword blocking, avoids provider-specific hacks, and avoids
prematurely refactoring the full memory pipeline.

## Validation Plan

- write failing tests for compiler-level governed rendering and shared advisory
  sanitization reuse
- confirm red tests fail for the right reason before implementation
- implement the smallest prompt-fragment policy extension and shared renderer
- run targeted tests for discovery delta, advisory memory, and compiler output
- run fmt, clippy, and at least focused app-layer regression suites before any
  broader workspace verification

## Expected Outcome

After this change:

- prompt fragments explicitly declare whether they are trusted or governed
  advisory content
- compiler-owned rendering rules enforce advisory demotion consistently
- `tool_discovery_delta` and advisory memory stop maintaining separate trust
  behavior
- future advisory prompt sources get a safe default path instead of requiring
  bespoke sanitization logic
