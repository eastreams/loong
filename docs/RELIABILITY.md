# Reliability

Reliability expectations and invariants for LoongClaw.

## Build Invariants

These must hold at every commit on every branch:

1. `cargo fmt --all -- --check` passes
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes
3. `cargo test --workspace --all-features` passes (currently 242+ tests)

Enforced by: CI (`verify` workflow). The optional `scripts/pre-commit` hook runs a subset of these checks locally.

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
