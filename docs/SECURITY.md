# Security

Security domain index for LoongClaw. For vulnerability reporting, see [SECURITY.md](../SECURITY.md) at repository root.

## Security Model

LoongClaw implements a multi-layer security model. Higher layers add defense-in-depth:

| Layer | Mechanism | Version | Status |
|-------|-----------|---------|--------|
| 0 | Rust memory safety (compile-time, zero overhead) | v0.1 | Enforced |
| 1 | Capability-based access (type-system tokens) | v0.1 | Enforced |
| 2 | Namespace confinement (per-task resource view) | v0.1 | Struct exists, not enforced |
| 3 | WASM linear memory sandbox | v0.2 | Research only |
| 4 | Process isolation (seccomp+Landlock / restricted child) | v0.1 | Not implemented |

## Enforcement Points

### Policy Engine (L1)

Every kernel-bound core tool call passes through capability + policy gates:

```
CapabilityToken → PolicyEngine.authorize(...) → PolicyExtensionChain → Execution → Audit
```

Tool-specific request approval currently lives in the `PolicyExtensionChain`; the legacy
`PolicyEngine::check_tool_call` hook is deprecated.

**Current coverage:**
- `shell.exec` — Kernel-mediated core tool execution with capability checks, shell policy extensions, and audit events
- `file.read` / `file.write` / `file.edit` — Kernel-mediated core tool execution with filesystem capabilities, file policy extension checks, execution-layer path sandboxing, and audit events
- Conversation tool turns — Fast-lane and safe-lane inner tool execution now flow through an explicit `ConversationRuntimeBinding` (`Kernel` or `Direct`); core tools require a bound `KernelContext`, missing authority is rejected at the binding boundary as `no_kernel_context`, and async delegate child turns now inherit parent kernel authority instead of forcing direct mode
- Memory/runtime/context orchestration — The conversation module now carries `ConversationRuntimeBinding` end-to-end across runtime, context, persistence, turn coordination, loop followup, history, and app-dispatch seams. Kernel-bound history readers fail closed on kernel memory-window errors or non-`ok` statuses instead of silently downgrading to direct sqlite
- Provider request/failover orchestration — Provider request entrypoints and failover telemetry now use an explicit `ProviderRuntimeBinding` (`Kernel` or `Direct`). Provider failover metrics record in both modes, while kernel-backed audit emission only occurs when provider execution is explicitly kernel-bound
- Outer integration wrappers — Raw optional kernel context is now limited to explicit integration boundaries such as `channel::process_inbound_with_provider`, which immediately normalize into a binding-first runtime seam instead of carrying shadow authority semantics deeper into the runtime
- Connector/ACP/runtime-only analytics — Not uniformly routed through the L1 policy chain yet

**Conversation runtime binding note:**
- The binding makes the high-level execution mode explicit: `Kernel` means the turn is allowed to call kernel-mediated core tools; `Direct` means conversation orchestration may continue, but kernel-only tool execution must fail closed.
- This removes ambiguity from conversation traits and dispatcher seams where `None` previously overloaded multiple meanings such as "direct mode", "not wired yet", or "forgot to pass kernel authority".
- Detached async delegate spawns carry an owned kernel context forward when the parent binding is kernel-bound. Direct-mode parents keep direct-mode children.
- Kernel-bound history helpers no longer reuse direct sqlite fallback behind the caller's back. Higher-level orchestration may still choose how to handle the surfaced error.
- Safe-lane governor diagnostics now surface history load status and normalized error codes instead of silently collapsing kernel history failures into an undifferentiated "no history" state.
- User-facing chat diagnostics now preserve explicit `ConversationRuntimeBinding` semantics end-to-end. The discovery-first session-history path uses the same binding-first implementation internally. The public `Option<&KernelContext>` entrypoint remains an explicit compatibility wrapper instead of carrying shadow authority semantics deeper into the runtime.

**Provider runtime binding note:**
- The provider binding makes provider governance explicit without importing conversation-layer semantics into provider code. `Kernel` means failover/audit behavior may emit kernel-backed audit events; `Direct` means provider execution is intentionally running without that authority while still recording in-process failover metrics.

### Capability Tokens

- 9 capability types with generation-based revocation
- `AtomicU64` threshold: revoke all tokens with generation <= N
- TTL enforcement on every authorization check
- `membrane` field exists but not enforced (TD-003)

### Audit System

- 7 event kinds with atomic sequencing
- Production app runtimes default to durable JSONL retention via `[audit].mode = "fanout"`
- Default journal path: `~/.loongclaw/audit/events.jsonl`
- `LoongClawKernel::new()` and spec/test/demo helpers may still opt into explicit in-memory audit seams when side-effect-free snapshot reporting is required
- Explicit no-audit behavior remains opt-in only and should stay reserved for narrow fixture seams
- No HMAC chain for tamper evidence (TD-007)

### Web HTTP SSRF Guardrails

- `web.fetch`, `web.search`, and the shared browser-side URL validators intentionally build their HTTP clients with `reqwest::ClientBuilder::no_proxy()`
- This keeps DNS resolution and connect-time routing inside the same SSRF-safe policy boundary instead of delegating host decisions to ambient `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, or `NO_PROXY` environment settings
- Trade-off: corporate proxy-only egress is not currently supported for these web tools because a proxy hop would weaken the same-host assumptions behind the private-address guard
- If proxy-aware web tooling is added later, it should preserve the same SSRF guarantees rather than silently bypassing them

### Compile-Time Constraints

25 workspace clippy denies prevent common agent anti-patterns. See [Harness Engineering](design-docs/harness-engineering.md) for the full list.

## See Also

- [Design Docs Index](design-docs/index.md) — security-related design decisions
- [Layered Kernel Design](design-docs/layered-kernel-design.md) — L1 security layer specification
- [Core Beliefs](design-docs/core-beliefs.md) — principle #3: capability-gated by default
