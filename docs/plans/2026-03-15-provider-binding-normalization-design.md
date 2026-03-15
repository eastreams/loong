# Provider Binding Normalization Design

Date: 2026-03-15
Scope: follow-up kernel-first refactor after issues #15, #154, #157, #167

## Problem

Conversation orchestration is now explicit about runtime authority through
`ConversationRuntimeBinding`, but the provider layer still collapses that
authority back into raw `Option<&KernelContext>`.

The remaining seams are concentrated in:

1. `provider::request_completion`
2. `provider::request_turn`
3. `provider::request_turn_in_view`
4. `request_failover_runtime::request_across_model_candidates`
5. `failover_telemetry_runtime::record_provider_failover_audit_event`

That means a provider caller still has to overload `None` with several
different meanings:

1. intentional direct provider execution
2. missing kernel authority for governed failover telemetry
3. an outer boundary that has not committed to either mode yet

This weakens the kernel-first architecture story exactly where retry/failover
behavior and audit authority should be explicit.

## Goals

1. Replace raw optional-kernel provider request seams with an explicit provider
   binding type.
2. Preserve current provider behavior:
   - direct provider execution still works
   - failover metrics still record in both modes
   - kernel-backed audit events still emit when authority is present
3. Keep the slice narrow and reviewable by avoiding broader provider abstraction
   redesign in this PR.

## Non-goals

1. Do not redesign provider transport, payload shaping, or model selection.
2. Do not fold the broader provider abstraction debt from issue #15 into this
   slice.
3. Do not sweep outer integration boundaries such as channel entrypoints unless
   a trivial wrapper cleanup naturally falls out.

## Alternatives Considered

### A. Keep `Option<&KernelContext>` in provider code

Rejected. The API shape remains ambiguous and reintroduces the same shadow-path
problem already being removed from conversation orchestration.

### B. Reuse `ConversationRuntimeBinding` directly inside provider

Rejected. It would leak conversation-layer semantics into provider code and
invert the dependency story. Provider request/failover logic should describe its
own authority contract instead of depending on a higher-level orchestration
binding type.

### C. Introduce a provider-specific explicit binding

Recommended. It keeps the authority contract explicit while preserving a clean
layer boundary: conversation code translates into provider semantics at the
conversation-to-provider seam, and provider internals no longer need to infer
meaning from `Option<&KernelContext>`.

## Proposed Design

Add a provider-scoped binding type, `ProviderRuntimeBinding`, with the same
two-state shape as the conversation binding but provider-specific semantics:

1. `Kernel` means provider failover/audit behavior is running with kernel-backed
   authority.
2. `Direct` means provider execution is intentionally running without that
   authority.

The provider layer uses the binding only where it is architecturally relevant:

1. provider request entrypoints accept `ProviderRuntimeBinding`
2. request failover orchestration forwards the binding without converting it
3. failover telemetry converts to `binding.kernel_context()` only at the audit
   emission leaf

## Why Not Share One Generic Binding Type?

This slice should not introduce a new generic runtime-binding abstraction across
multiple modules. That would expand the change from a clean provider follow-up
into a broader cross-layer design exercise.

Using separate explicit binding types keeps semantics local:

1. conversation binding answers "is this turn kernel-bound or direct?"
2. provider binding answers "is this provider request governed by kernel-backed
   failover/audit authority or direct?"

Those answers are related, but they are not the same contract.

## Translation Boundary

The conversion should happen at the existing conversation-to-provider seam in
`conversation/runtime.rs`.

That keeps responsibilities clean:

1. conversation runtime remains authoritative for turn execution mode
2. provider runtime becomes authoritative for provider request governance mode
3. outer integration boundaries such as `channel::process_inbound_with_provider`
   can stay optional until they reach a binding-first seam

## Expected Behavioral Outcome

Behavior should remain stable:

1. direct provider requests continue to work for direct conversation/runtime
   flows
2. provider failover metrics still record in both direct and kernel-bound modes
3. provider failover audit events still emit only when kernel authority is
   present
4. conversation runtime no longer has to collapse an explicit binding back into
   an optional kernel reference before calling provider code

The change is architectural clarity, not a new provider policy.

## Test Strategy

Add or adapt tests that prove:

1. public provider request APIs still support direct execution using the new
   explicit binding
2. failover audit events are still emitted when provider execution is
   kernel-bound
3. failover metrics still record when provider execution is direct
4. conversation-runtime provider calls still preserve direct-versus-kernel-bound
   behavior after translation
5. downstream crates that call the public provider APIs, such as daemon import
   tests, are updated to the explicit binding contract

## Why This Slice Matters

Kernel-first architecture is not just about routing core tools through the
kernel. It also requires the remaining authority-bearing call boundaries to be
explicit about when they are governed and when they are intentionally direct.

Finishing this provider seam removes one of the last raw optional-kernel
contracts from the main `app` runtime path and sets up later ACP/channel work on
top of a cleaner, more uniform inner runtime contract.
