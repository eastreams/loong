# Reliability

Reliability expectations and invariants for LoongClaw.

## Build Invariants

These must hold at every commit on every branch:

1. `cargo fmt --all -- --check` passes
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes
3. `cargo test --workspace` passes
4. `cargo test --workspace --all-features` passes (test count evolves with the codebase; CI is the source of truth)

Enforced by: CI (`.github/workflows/ci.yml`, surfaced through the aggregate `build` check). The
optional `scripts/pre-commit` hook mirrors these cargo gates locally.

## Runtime Stability Guardrails

1. **Wasm trap behavior is platform-aware by default** — on macOS, `signals_based_traps` is disabled to avoid trap-handler abort instability under parallel bridge tests.
2. **Runtime override is explicit** — set `LOONGCLAW_WASM_SIGNALS_BASED_TRAPS=true|false` to force trap behavior for diagnostics/experiments.
3. **Daemon stress helper is scriptable** — run `./scripts/stress_daemon_tests.sh 10 default,2,1` for manual repeated daemon test validation across thread modes.
4. **Trap-mode matrix is available when needed** — set `LOONGCLAW_STRESS_WASM_TRAPS_MODES=auto,false,true` to sweep daemon tests across trap behavior modes during targeted investigation.

## Architecture Stability Guardrails

1. **Complexity budgets are locally machine-checkable** — run `./scripts/check_architecture_boundaries.sh` or `task check:architecture` to inspect module line/function budgets for architecture hotspots (`spec_runtime`, `spec_execution`, `provider/mod`, `memory/mod`).
2. **Memory operation literals are boundary-guarded** — memory core operation strings (`append_turn`, `window`, `clear_session`) must remain centralized in `crates/app/src/memory/*` and never spread into callsites.
3. **`spec` stays detached from `app`** — the architecture guardrails treat any direct `loongclaw-app` dependency in `crates/spec/Cargo.toml` as a boundary regression, and `./scripts/check_dep_graph.sh` must stay green.
4. **Strict enforcement is an extended local gate** — use `task check:architecture:strict` (or set `LOONGCLAW_ARCH_STRICT=true`) to make architecture budget violations fail non-zero. This check is part of `task verify:full`, not the canonical CI-parity gate.

## Kernel Invariants

1. **Token authorization is fail-closed** — if the policy engine cannot determine authorization (e.g., mutex poisoned), the operation is denied.
2. **Audit events are never silently dropped** — production-shaped CLI chat, Telegram, and Feishu bootstraps default to `audit.mode = "fanout"`, which appends `~/.loongclaw/audit/events.jsonl` and can retain in-memory snapshots for local diagnostics. `LoongClawKernel::new()` now defaults to `InMemoryAuditSink`, `spec` bootstrap/runner helpers intentionally use a named in-memory audit helper for side-effect-free snapshot reporting, and `NoopAuditSink` remains reserved for callers that explicitly opt into `new_without_audit(...)` or wire a noop sink themselves.
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
