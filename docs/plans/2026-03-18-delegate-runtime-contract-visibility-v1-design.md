# Delegate Runtime Contract Visibility V1 Design

**Problem**

`#280` and `#281` made delegate-child runtime narrowing real at execution time for kernel-bound
core tools, but the child planning surface still does not explain that effective contract clearly to
the model before it plans.

Today LoongClaw can already persist and enforce a child runtime posture through:

- the constrained delegate execution envelope
- child-session lifecycle events
- `SessionContext.runtime_narrowing`
- trusted internal `_loongclaw.runtime_narrowing`
- `ToolRuntimeConfig::narrowed(...)`

That closes the execution-time correctness gap, but it still leaves a planning-time visibility gap:

- the child can only discover some limits after a tool call fails
- the model may plan as if the root web/browser posture still applies
- operators do not get a prompt-level statement of the contract that is already being enforced

This slice addresses that narrower usability gap without widening the runtime-policy surface.

**Goal**

Surface the effective delegate child runtime contract directly in child planning prompts so the model
can plan against the same web/browser posture that the runtime will enforce.

The target behavior is:

1. Child sessions with non-empty persisted `runtime_narrowing` receive a stable prompt summary.
2. Root sessions and unrestricted sessions remain unchanged.
3. The prompt summary reflects the same narrowing semantics already enforced for:
   - `web.fetch`
   - `browser.*`
4. No new planning channel or schema mutation is introduced.

**Non-Goals**

- Do not change actual runtime enforcement semantics.
- Do not mutate provider tool schemas based on session-specific policy.
- Do not add a new planning metadata envelope or side channel.
- Do not expand the contract to shell, filesystem, or external-skills policy in this slice.
- Do not redesign provider request assembly.

**Root Cause**

The real defect is not missing enforcement. That was the previous slice.

The remaining defect is that the model does not see the effective child runtime contract early enough
in the prompt assembly path. The existing runtime narrowing is available on `SessionContext`, but the
system prompt builder does not project it into planning-visible text.

As a result, the child can still reason from a broader imagined capability surface and only learn the
real limits through execution failure.

**Approaches Considered**

1. Add child-specific text to provider tool definitions.
   Rejected because tool definitions are effectively static provider-facing schema. Injecting
   per-session runtime posture there would blur the boundary between static tool metadata and dynamic
   execution policy.

2. Add a separate internal planning metadata channel.
   Rejected because it introduces a second mechanism when the existing system prompt rewrite path is
   already sufficient for this slice.

3. Reuse the existing system prompt rewrite path and append a child-only runtime contract summary.
   Recommended because it is the smallest correct change:
   - the child contract is already available on `SessionContext`
   - the system prompt is already session-scoped
   - the change stays local to prompt assembly
   - no provider schema or execution pipeline changes are needed

**Chosen Design**

Add a stable formatter on `ToolRuntimeNarrowing` that produces a deterministic system-prompt block.

The block will be injected only when all of the following are true:

- the session is a child session
- `SessionContext.runtime_narrowing` is present
- the narrowing is not empty
- system-prompt inclusion is enabled

The block should be explicit that these are hard runtime limits for the child session, not generic
guidance.

Recommended shape:

```text
[delegate_child_runtime_contract]
Plan within these child-session runtime limits:
- web.fetch private hosts: denied
- web.fetch allowed domains: docs.example.com
- web.fetch blocked domains: deny.example.com
- web.fetch timeout seconds: 5
- web.fetch max bytes: 4096
- web.fetch max redirects: 2
- browser max sessions: 1
- browser max links: 8
- browser max text chars: 512
Treat these as enforced limits for this child session.
```

Only fields present in the narrowing contract should appear. Empty sections should be omitted. The
marker and ordering must be deterministic for stable prompting and testing.

**Injection Path**

Use `DefaultConversationRuntime::build_context(...)` as the injection seam.

Why this seam is correct:

- it already resolves `SessionContext`
- it already rewrites the effective system prompt
- it is the narrowest session-scoped prompt assembly path
- it avoids duplicating logic in lower provider request builders that do not naturally own child
  runtime context

Implementation outline:

1. Resolve child `SessionContext`.
2. Derive `delegate_runtime_contract_prompt_summary()` from `runtime_narrowing`.
3. Merge that summary with any existing `system_prompt_addition`.
4. Reuse `apply_system_prompt_addition(...)` to prepend the merged addition to the system prompt.

This keeps the change additive and localized.

**Why Not Use `apply_tool_view_to_system_prompt(...)`**

`apply_tool_view_to_system_prompt(...)` is aimed at projecting tool visibility, not runtime policy.
It rewrites the capability snapshot section, while the delegate runtime contract is better modeled as
separate planning guidance that accompanies the system prompt rather than replacing the snapshot.

Using the system-prompt addition path keeps tool visibility and runtime-policy visibility as distinct
concerns.

**Testing Strategy**

Write failing tests first for:

- child build-context prompt includes the runtime contract marker and concrete narrowed values
- root build-context prompt does not include the child contract marker
- child sessions with empty narrowing do not inject the block
- the formatter emits stable ordered lines for web/browser narrowing fields

Then run the focused conversation/runtime and runtime-config tests before broader repository
verification.

**Risk Assessment**

The main risk is semantic drift between:

- the narrowing contract used for enforcement
- the prompt summary shown to the model

This is controlled by formatting directly from the typed `ToolRuntimeNarrowing` struct rather than
reconstructing the prompt contract from unrelated config or free-form strings.

The second risk is accidental prompt noise. This is controlled by:

- child-only gating
- non-empty narrowing gating
- field omission for unset values
- deterministic compact formatting

**Why This Slice Is Worth Doing**

The previous slice solved correctness at execution time. This slice improves correctness at planning
time using the same contract.

That means fewer avoidable failed tool calls, clearer operator expectations, and tighter alignment
between what the child can plan and what it can actually execute.
