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
- `shell.exec` — Full policy check (allowlist, denylist, approval gates)
- `file.read` / `file.write` — Path sandboxing only, no policy engine check (TD-002)
- Runtime/memory/connector — No policy check

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

## Critical Security Gaps

These are tracked in the [Tech Debt Tracker](tech-debt-tracker.md):

| Priority | Gap | TD ID |
|----------|-----|-------|
| High | Policy engine only gates `shell.exec` | TD-002 |
| High | Audit events in-memory only | TD-006 |
| High | No HMAC chain on audit events | TD-007 |
| Medium | Membrane field never checked | TD-003 |
| Medium | Namespace struct not enforced | TD-005 |
| Medium | No WASM fuel metering | TD-013 |
| Medium | Plugin scanner logic absent | TD-012 |

## Research Decisions

Security-related decisions from the research repository:

| ID | Decision | Status |
|----|----------|--------|
| D-001 | Zircon-style capability model | Partially implemented |
| D-013 | Three-tier capability negotiation | Research |
| D-015 | OAuth 2.1 external + capability internal | Research |
| D-025 | Per-invocation plugin isolation | Research |
| D-030 | Zero-capability-default WASI injection | Research |

## See Also

- [Harness Engineering](design-docs/harness-engineering.md) — backpressure and constraint model
- [Layered Kernel Design](design-docs/layered-kernel-design.md) — L1 security layer specification
- [Core Beliefs](design-docs/core-beliefs.md) — principle #3: capability-gated by default
