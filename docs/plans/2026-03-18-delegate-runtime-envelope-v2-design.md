# Delegate Runtime Envelope V2 Design

**Problem**

`feat/constrained-subagent-v1` made delegate children explicit enough to be inspectable:

- child sessions now persist a typed constrained-subagent execution envelope
- depth and active-child limits are enforced at spawn time
- session inspection can explain the launch contract after the fact

But the execution envelope still stops at the session boundary.

Today a child session can have a narrower visible tool surface while still executing core tools with
the process-global runtime policy because kernel-bound tool execution ultimately routes through:

- `TurnEngine::execute_tool_intent_via_kernel(...)`
- `KernelContext`
- `MvpToolAdapter::new()`
- `execute_tool_core(...)`
- `get_tool_runtime_config()`

That means LoongClaw can currently create a child that *looks* constrained in:

- `tool_view`
- provider-visible tool definitions
- `session_status`

while still letting `web.fetch` / `browser.*` run with the parent process runtime posture.

This is the next real architectural gap after the first constrained-subagent slice.

**Goal**

Add a second bounded slice that makes delegate child runtime posture real for kernel-bound core tool
execution without introducing a separate subagent runtime or kernel control plane.

The target for this slice is:

1. Define a typed child runtime-narrowing contract for delegate executions.
2. Persist that contract inside the existing constrained-subagent execution envelope.
3. Carry the contract into trusted internal tool context during child-session tool execution.
4. Narrow actual core-tool runtime config at execution time for child sessions.
5. Cover the first meaningful policy surfaces:
   - `web.fetch` domain/private-host posture and transport limits
   - `browser.*` session/text/link limits

**Non-Goals**

- Do not add a new global subagent scheduler, queue, or registry.
- Do not redesign ACP routing or introduce a subagent-specific kernel pack model.
- Do not solve context-budget shaping in this slice.
- Do not widen the child runtime contract to every tool class at once.
- Do not bypass kernel execution for child tools.

**Root Cause**

The first constrained-subagent slice made child launches explicit, but the effective runtime policy
for core tools is still resolved at the wrong layer.

The root problem is not missing allowlist fields. The root problem is that the actual kernel-bound
tool execution path has no session-scoped runtime-policy carrier. The system currently narrows:

- what a child can *see*
- what a child session reports about itself

but it does not narrow:

- what runtime config `execute_tool_core_with_config(...)` sees once a kernel-bound child tool call
  actually executes

So the missing contract is not “more delegate knobs”. The missing contract is “how a child
session’s runtime posture survives the trip into trusted core-tool execution”.

**External Reference**

OpenClaw and Codex both treat subagents as more than prompt forks:

- OpenClaw emphasizes bounded depth, explicit lifecycle, and child-session governance.
- Codex emphasizes isolated execution environments and async child-session continuity.

LoongClaw is not ready for their full orchestration surface yet, but it does need one key property
they both rely on: child execution constraints must remain true at actual execution time, not only
at planning time.

That makes runtime-envelope propagation the right next slice.

**Approaches Considered**

1. Only widen `ToolView` / `child_tool_allowlist`.
   Rejected because it preserves the current false boundary: visibility would narrow, but
   `execute_tool_core(...)` would still run under the global runtime config.

2. Create a child-specific `KernelContext` with its own tool adapter and policy extensions.
   Rejected for this slice because it would force a broader redesign of kernel bootstrap,
   audit/plumbing continuity, and per-child adapter registration. It solves the problem, but at the
   wrong cost right now.

3. Store child runtime narrowing in trusted internal tool context and derive an effective runtime
   config inside core-tool execution.
   Recommended because it:
   - keeps kernel execution intact
   - avoids mutable global runtime state
   - reuses the existing reserved `_loongclaw` trusted payload seam
   - lets one typed contract drive persistence, inspection, and actual execution

**Chosen Design**

Add a typed runtime-narrowing contract under the existing constrained-subagent execution envelope.

The contract will include:

- `web_fetch`
  - `allow_private_hosts: Option<bool>`
  - `allowed_domains`
  - `blocked_domains`
  - `timeout_seconds: Option<u64>`
  - `max_bytes: Option<usize>`
  - `max_redirects: Option<usize>`
- `browser`
  - `max_sessions: Option<usize>`
  - `max_links: Option<usize>`
  - `max_text_chars: Option<usize>`

The contract is a *narrowing* shape, not a second full runtime config.

Semantics:

- booleans can only stay the same or become more restrictive
- numeric limits clamp downward
- blocked-domain sets union with parent policy
- allow-domain sets intersect with parent allowlists when both are present
- an empty child allow-domain list means “no additional allowlist narrowing”

This keeps the slice monotonic: child posture can only stay within or below the parent runtime
policy.

**Configuration Shape**

Add a nested delegate child runtime policy section under `tools.delegate`:

- `tools.delegate.child_runtime.web`
- `tools.delegate.child_runtime.browser`

This keeps runtime narrowing local to delegate execution instead of leaking a subagent-specific
schema across unrelated tool config.

Examples of intended use:

- limit child browser concurrency to one session even if root allows more
- allow child web access only to a documentation domain
- force child web requests to stay off private hosts even when the root runtime allows them

**Execution Path**

For a child session:

1. `execute_delegate_tool(...)` or `execute_delegate_async_tool(...)` builds the persisted
   `ConstrainedSubagentExecution`, including `runtime_narrowing`.
2. `ConversationRuntime::session_context(...)` resolves the child’s persisted execution envelope and
   exposes the narrowing contract on `SessionContext`.
3. `TurnEngine` injects trusted internal tool context for child core-tool calls.
4. `execute_tool_core_with_config(...)` reads the trusted internal runtime narrowing and derives an
   effective `ToolRuntimeConfig`.
5. `web.fetch`, `browser.*`, and tool-search runtime filtering execute against the narrowed config.

This keeps one execution contract flowing through:

- persisted lifecycle events
- in-memory session context
- trusted core-tool payload context
- actual executor runtime config

**Why Trusted Internal Tool Context**

LoongClaw already reserves `_loongclaw` for trusted internal payload context and rejects untrusted
callers that try to forge it.

Using that seam is the smallest durable option because:

- it is already the right place for execution-only metadata
- it avoids broadening public tool schemas
- it composes with `tool.invoke`
- it keeps session-specific runtime posture off global singleton state

**Session Inspection Changes**

`session_status` already surfaces the constrained-subagent envelope. This slice extends that
envelope with `runtime_narrowing` so the inspection view answers:

- which web/browser runtime posture governed the child
- whether the child was stricter than the root runtime
- whether the child was prevented from private-host access or broad domain access

Inspection must continue to read the persisted snapshot rather than recomputing from current config.

**Why This Is Smaller Than A Child Kernel Context**

The tempting alternative is a per-child kernel instance or per-child adapter selection. That would
be architecturally larger because it would force LoongClaw to answer, in one slice:

- how audit sink continuity works across child kernels
- how policy extensions are cloned or re-bound
- how async child execution transports kernel state
- how nested children compose multiple kernel instances

LoongClaw does not need those answers yet to make child runtime posture real. It only needs a
session-scoped execution contract that survives to actual core-tool execution.

**Testing Strategy**

Add failing tests first for:

- child `session_status` exposing persisted runtime narrowing
- child kernel-bound `web.fetch` being denied by narrowed domain/private-host policy even when the
  base runtime is broader
- child kernel-bound `browser.open` / `browser.extract` obeying narrowed browser limits
- untrusted payloads being unable to forge runtime narrowing
- runtime narrowing never widening parent policy

Then run adjacent delegate/session/tool regressions and full repository verification.

**Risk Assessment**

The main risk is semantic drift between:

- persisted runtime narrowing
- injected trusted payload context
- effective runtime config merging logic

That risk is controlled by:

- using one typed narrowing contract
- keeping the merge logic centralized in `ToolRuntimeConfig`
- testing both persisted inspection and actual tool execution

The other risk is overfitting the contract to one tool. This design avoids that by targeting the
shared runtime-policy layer (`ToolRuntimeConfig`) instead of adding one-off checks inside delegate
or provider code.
