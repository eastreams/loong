# Core Beliefs

These are the golden principles for anyone — agent or human — working in this codebase. They encode architectural taste and are enforced mechanically where possible.

1. **Kernel-first** — all execution paths (MVP, spec, bench) route through the kernel's capability/policy/audit system. No shadow paths that bypass policy.

2. **No breaking changes** — new primitives are additive. Existing public APIs keep their signatures. Use `Option`, defaults, and new methods instead of modifying existing ones.

3. **Capability-gated by default** — every tool call, memory operation, and connector invocation requires a valid `CapabilityToken` with matching capabilities granted by the pack manifest.

4. **Audit everything security-critical** — policy denials, token lifecycle events, and plane invocations all emit structured audit events. Silent drops are bugs (see: NoopAuditSink incident).

5. **7-crate DAG, no cycles** — `contracts` (leaf) -> `kernel` -> `protocol` (leaf) -> `app` -> `spec` -> `bench` -> `daemon`. Dependency direction is non-negotiable. See [Layered Kernel Design](layered-kernel-design.md).

6. **Tests are the contract** — if a behavior isn't tested, it doesn't exist. All tests pass at every commit, enforced by the pre-commit hook.

7. **Boring technology preferred** — choose well-understood, composable dependencies that agents can reason about from repo context alone. Reimplement small utilities rather than pulling in opaque upstream packages.

8. **Repository is the system of record** — design decisions, plans, and architectural context live in `docs/`, not in chat threads or people's heads. If it's not in the repo, it doesn't exist for agents.

9. **Enforce mechanically, not manually** — prefer linters, CI gates, and pre-commit hooks over code review comments. Encode taste into tooling. See `scripts/pre-commit`.

10. **YAGNI ruthlessly** — don't design for hypothetical future requirements. The minimum complexity for the current task is the right amount. Three similar lines of code is better than a premature abstraction.
