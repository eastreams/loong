# Delegate Runtime Effective Contract Alignment Design

**Problem**

The current delegate child prompt summary is derived directly from the persisted
`ToolRuntimeNarrowing`.

That looks reasonable at first glance, but it is not the same contract that execution uses.
Execution applies child narrowing through:

```text
effective_runtime = base_tool_runtime_config.narrowed(&runtime_narrowing)
```

The prompt layer currently skips that last step and therefore has a semantic drift window.

Examples of drift:

- `allow_private_hosts: Some(true)` is not a widening grant. It means "do not narrow this field".
  Rendering that as `allowed` is wrong whenever the base runtime already denies private hosts.
- child `allowed_domains` can collapse to an empty intersection against the base runtime allowlist,
  which makes `web.fetch` fail closed, but the prompt can still show the requested child domains.
- browser and web numeric ceilings are clamped against the base runtime. The prompt should show the
  clamped effective result, not only the requested child value.

This is a deeper correctness bug than missing prompt visibility. It means the new visibility layer
can still describe a broader contract than the child can actually execute.

**Goal**

Render the delegate child prompt summary from the effective runtime contract seen by execution,
while keeping the change local to the existing prompt-assembly seam.

The desired behavior is:

1. Prompt summaries remain child-only and only appear when persisted `runtime_narrowing` is
   non-empty.
2. Prompt values reflect the same effective runtime posture that
   `execute_tool_core_with_config(...)` will enforce.
3. Fail-closed outcomes caused by narrowing interaction, especially empty allowlist intersections,
   are visible to the model instead of silently disappearing.
4. The fix does not add a new metadata channel or widen the scope into a generic runtime-dashboard
   system.

**Non-Goals**

- Do not redesign child runtime narrowing semantics.
- Do not add new delegate child config fields.
- Do not introduce a new shared "runtime contract snapshot" layer for every UI or inspection
  surface in this slice.
- Do not change provider tool schemas.
- Do not change session-status JSON shape in this slice.

**Root Cause**

The original visibility slice fixed the wrong abstraction boundary.

`ToolRuntimeNarrowing` is an input to effective-policy derivation, not the effective policy itself.
It carries "requested child tightening", not "final executable contract". Prompt formatting from
that input is only safe when the base runtime is always weaker or equal, which is not guaranteed.

The real invariant we need is:

```text
prompt-visible child contract == execution-time effective child contract
```

That invariant can only hold if the prompt summary is derived after the same narrowing logic used by
tool execution.

**Approaches Considered**

1. Patch the obvious broken fields only.
   Rejected because it would still leave drift for empty allowlist intersections, blocked-domain
   unions, and numeric clamping. That is a local patch, not a root-cause fix.

2. Derive the prompt summary from the effective `ToolRuntimeConfig` plus the child narrowing inputs.
   Recommended because it is the smallest correct fix:
   - reuse existing narrowing semantics
   - stay inside the current conversation/runtime seam
   - avoid inventing a new shared policy-snapshot subsystem
   - make prompt behavior match execution behavior

3. Create a first-class effective runtime contract snapshot model shared by prompt assembly, session
   status, and diagnostics.
   Rejected for now because the abstraction is reasonable long term but too large for this bugfix
   slice. It would expand surface area and review scope beyond the current issue.

**Chosen Design**

Keep the child-only prompt injection seam in `DefaultConversationRuntime::build_context(...)`, but
replace raw narrowing formatting with effective-contract formatting.

Recommended API shape:

```rust
impl ToolRuntimeConfig {
    pub fn delegate_child_prompt_summary(
        &self,
        narrowing: &ToolRuntimeNarrowing,
    ) -> Option<String>;
}
```

Behavior:

1. Return `None` when `narrowing.is_empty()`.
2. Compute `effective = self.narrowed(narrowing)`.
3. Render only child-runtime-relevant fields, but use effective values.
4. Surface fail-closed effective outcomes explicitly when narrowing requested an allowlist but the
   effective intersection is empty.

**Prompt Formatting Rules**

- Keep deterministic line ordering.
- Keep the existing marker so tests and prompt consumers stay stable:

```text
[delegate_child_runtime_contract]
```

- Render effective values, not requested values.
- Omit disabled tool surfaces when tool visibility already hides them.
- When `allowed_domains` was requested by the child but the effective allowed-domain set is empty,
  render an explicit fail-closed line:

```text
- web.fetch allowed domains: none (effective intersection is empty)
```

This is important because omission would hide the fact that `web.fetch` is still visible in the
tool surface but unusable for outbound fetches.

**Why This Is Still Minimal**

The recommended change does not alter:

- the persisted delegate execution envelope
- trusted internal payload injection
- tool execution narrowing
- tool visibility calculation

It only changes the source of truth used to format prompt-visible child contract text.

**Testing Strategy**

Write failing tests first for:

1. `ToolRuntimeConfig::delegate_child_prompt_summary(...)` renders effective values when the base
   runtime is stricter than the child request.
2. An empty allowlist intersection is surfaced explicitly instead of showing the requested domains.
3. `DefaultConversationRuntime::build_context(...)` uses the effective summary path in child
   prompts.
4. Existing root-session and empty-child negative controls remain green.

**Risk Assessment**

The main risk is accidentally surfacing too much base runtime detail and turning this summary into a
general runtime dump. That is controlled by only rendering fields tied to non-empty child narrowing
inputs.

The second risk is changing previously accepted prompt strings. That is intentional and correct
because the current strings can be false. Deterministic tests should lock the new behavior down.

**Why This Slice Is Worth Doing**

If LoongClaw says a constrained child may fetch `docs.example.com` while the actual runtime will
fail closed, the visibility layer is actively misleading.

This fix restores the stronger invariant:

```text
what the child plans against == what the child can execute
```
