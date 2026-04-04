# Tool-Discovery Prompt Boundary Design

Date: 2026-04-01
Epic: #758
Related PR: #771
Status: approved for implementation

## Goal

Harden the boundary between advisory `tool.search` discovery state and the
authoritative system prompt so persisted discovery text cannot masquerade as
fresh runtime instructions while preserving the follow-up utility of recent
discovery context.

## Current State

LoongClaw persists a lease-free subset of `tool.search` output as the
`tool_discovery_refreshed` conversation event.

That state is later rehydrated and projected back into the system prompt as the
`[tool_discovery_delta]` fragment.

The remaining gap is that `query`, `diagnostics.reason`, `summary`,
`search_hint`, `argument_hint`, `required_fields`, and
`required_field_groups` are currently interpolated into prompt text almost
verbatim.

This means advisory search output can regain higher apparent authority when it
is rendered as system-level prose.

There is also a smaller state-recovery gap:

- `ToolDiscoveryState::from_tool_search_payload(...)` does not treat
  `results`-only payloads as valid state when `query`, `exact_tool_id`,
  `diagnostics`, and `returned` are absent

And there is a model-followup consistency gap:

- summary compaction drops top-level discovery metadata such as
  `exact_tool_id` and `diagnostics`, even though those fields are still useful
  in the immediate follow-up provider round

## Non-Goals

- do not redesign the prompt fragment architecture
- do not remove discovery-delta follow-up guidance
- do not change `tool.search` result schema beyond what is needed for safe
  persistence and rendering
- do not broaden this slice into unrelated middleware or provider rewrites

## Options Considered

### Option 1: escape everything into JSON strings

Render the entire discovery state as JSON or quoted blobs.

This would be safe, but it would reduce readability for the model and degrade
the value of the discovery-delta follow-up guidance.

### Option 2: sanitize advisory fields and keep structured prose

Keep the current discovery-delta shape.

Normalize untrusted text into safe single-line advisory values before rendering
them into the system prompt.

Preserve the high-value fields and refresh guidance.

This is the recommended option because it fixes the trust boundary without
rewriting prompt topology.

### Option 3: stop projecting discovery state into the system prompt

Keep the persisted event but never rehydrate it into prompt space.

This is the safest option, but it would undo the feature value introduced for
issue `#758`.

## Chosen Design

Use option 2.

Add a small discovery-delta rendering guard inside the existing
`ToolDiscoveryState` prompt renderer.

The guard should:

- treat discovery text as advisory data, not instructions
- collapse multi-line or control-heavy content into safe single-line values
- prevent raw markdown headings, code fences, XML-like tags, or line breaks from
  being projected as separate prompt structure
- preserve visible tool ids and refresh guidance

This design keeps the existing prompt fragment lane and state model.

It changes only how untrusted discovery text is admitted into prompt space.

## Scope of Code Changes

### 1. Discovery-state parsing hardening

Update `ToolDiscoveryState::from_tool_search_payload(...)` so payloads with
valid `results` entries still produce advisory state even when other top-level
fields are absent.

### 2. Discovery-delta rendering hardening

Add a narrow sanitizer for advisory text fields used by
`render_delta_prompt(...)`.

The sanitizer should:

- trim outer whitespace
- replace internal newlines and other control spacing with plain spaces
- neutralize markdown-heading shape at field starts
- avoid introducing quoting or escaping noise when unnecessary

### 3. Compaction metadata consistency

Update `compact_tool_search_payload_summary(...)` so model-facing compact
summaries preserve the advisory metadata needed by the immediate follow-up turn.

Keep `lease` in compacted result entries because `tool.invoke` still depends on
the current search result lease during the same follow-up loop.

Keep persisted `tool_discovery_refreshed` state lease-free.

### 4. Regression coverage

Add red-green tests that prove:

- malicious multi-line discovery text is flattened before reaching the system
  prompt
- results-only payloads still hydrate advisory state
- compacted summaries preserve `exact_tool_id`, `returned`, and `diagnostics`
  while still keeping result-entry leases available for the live follow-up path
- existing exact-refresh guidance and tool-view filtering still work

## Why This Is Minimal and Correct

The root cause is not that discovery state exists.

The root cause is that advisory text from `tool.search` crosses into system
prompt space without a dedicated rendering boundary.

This design fixes that root cause where the trust boundary is crossed.

It avoids hardcoded query blocking, avoids deleting useful discovery context,
and stays aligned with the current prompt-fragment architecture.

## Validation Plan

- write failing tests first
- run targeted red tests and confirm the failure reason matches the boundary gap
- implement the smallest rendering and compaction fixes
- run targeted green tests
- run fmt, clippy, targeted tests, and full workspace tests

## Expected Outcome

After this change:

- discovery-delta remains available as advisory context
- persisted discovery text no longer re-enters prompt space as raw multi-line
  or structure-shaping instructions
- results-only payloads still recover useful advisory state
- compacted search summaries preserve immediate follow-up metadata without
  weakening the separate lease-free persisted discovery-state contract
