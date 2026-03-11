# Reliability

Reliability expectations and invariants for LoongClaw.

## Build Invariants

These must hold at every commit on every branch:

1. `cargo fmt --all -- --check` passes
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes
3. `cargo test --workspace --all-features` passes (test count evolves with the codebase; CI is the source of truth)

Enforced by: CI (`verify` workflow). The optional `scripts/pre-commit` hook runs a subset of these checks locally.

## Runtime Stability Guardrails

1. **Wasm trap behavior is platform-aware by default** — on macOS, `signals_based_traps` is disabled to avoid trap-handler abort instability under parallel bridge tests.
2. **Runtime override is explicit** — set `LOONGCLAW_WASM_SIGNALS_BASED_TRAPS=true|false` to force trap behavior for diagnostics/experiments.
3. **Daemon stress gate is scriptable** — run `./scripts/stress_daemon_tests.sh 10 default,2,1` to execute repeated daemon test rounds across thread modes.
4. **Trap-mode matrix is available when needed** — set `LOONGCLAW_STRESS_WASM_TRAPS_MODES=auto,false,true` to sweep daemon tests across trap behavior modes.

## Architecture Stability Guardrails

1. **Complexity budgets are machine-checkable** — run `./scripts/check_architecture_boundaries.sh` to inspect module line/function budgets for architecture hotspots (`spec_runtime`, `spec_execution`, `provider/mod`, `memory/mod`).
2. **Memory operation literals are boundary-guarded** — memory core operation strings (`append_turn`, `window`, `clear_session`) must remain centralized in `crates/app/src/memory/*` and never spread into callsites.
3. **Strict enforcement is opt-in for local hard gates** — set `LOONGCLAW_ARCH_STRICT=true` to make architecture budget violations fail non-zero.

## Kernel Invariants

1. **Token authorization is fail-closed** — if the policy engine cannot determine authorization (e.g., mutex poisoned), the operation is denied.
2. **Audit events are never silently dropped** — all bootstrap paths use `InMemoryAuditSink` or better. `NoopAuditSink` is reserved for tests that explicitly don't need audit.
3. **Pack registration is idempotent-safe** — duplicate pack IDs return `DuplicatePack` error, never silently overwrite.
4. **Generation-based revocation is monotonic** — the revocation threshold only increases, never decreases.
5. **TaskState transitions are irreversible from terminal states** — `Completed` and `Faulted` states cannot transition.

## MVP Channel Invariants

1. **Kernel context is bootstrapped at startup** — CLI chat, Telegram, and Feishu channels all create `KernelContext` before processing messages.
2. **Memory persistence failures are surfaced** — `persist_turn` errors propagate to the caller, never silently swallowed.
3. **Provider errors have two modes** — `Propagate` (return error) or `InlineMessage` (synthetic reply). Behavior is explicit per channel.

## Test Expectations

- Kernel crate: property tests (proptest) for capability boundary invariants
- All crates: deterministic tests (no time-dependent flakes)
- Multi-threaded tests use `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`
