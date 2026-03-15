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

Every tool call passes through capability + policy gates:

```
CapabilityToken → PolicyEngine → PolicyExtensionChain → Execution → Audit
```

**Current coverage:**
- `shell.exec` — Kernel-mediated tool execution with capability checks, shell policy extensions, and audit events
- `file.read` / `file.write` — Kernel-mediated tool execution with filesystem capabilities, file policy extension checks, and audit events
- Conversation tool turns — Fast-lane and safe-lane inner tool execution now require a bound `KernelContext`; missing kernel authority is rejected at the runtime binding boundary as `no_kernel_context`
- Memory/runtime/context orchestration — Partially kernelized; some surrounding traits still carry optional kernel context and remain architectural debt rather than full L1 enforcement
- Connector/ACP/runtime-only analytics — Not uniformly routed through the L1 policy chain yet

### Capability Tokens

- 9 capability types with generation-based revocation
- `AtomicU64` threshold: revoke all tokens with generation <= N
- TTL enforcement on every authorization check
- `membrane` field exists but not enforced (TD-003)

### Audit System

- 7 event kinds with atomic sequencing
- In-memory only (TD-006) — lost on restart
- No HMAC chain for tamper evidence (TD-007)
- No persistent audit sink

### Compile-Time Constraints

25 workspace clippy denies prevent common agent anti-patterns. See [Harness Engineering](design-docs/harness-engineering.md) for the full list.

## See Also

- [Design Docs Index](design-docs/index.md) — security-related design decisions
- [Layered Kernel Design](design-docs/layered-kernel-design.md) — L1 security layer specification
- [Core Beliefs](design-docs/core-beliefs.md) — principle #3: capability-gated by default
