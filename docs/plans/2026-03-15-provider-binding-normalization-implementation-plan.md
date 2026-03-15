# Provider Binding Normalization Implementation Plan

**Goal:** Replace the remaining raw optional-kernel provider request and
failover seams with an explicit provider runtime binding while preserving direct
provider execution and kernel-backed failover audit behavior.

**Architecture:** Add a provider-scoped `ProviderRuntimeBinding` type, thread it
through provider request entrypoints and failover telemetry helpers, and perform
the conversation-to-provider translation at `conversation/runtime.rs`. Keep
outer channel integration optional for now, and only convert back to
`Option<&KernelContext>` at the audit-emission leaf.

**Tech Stack:** Rust, Tokio tests, GitHub issue-first workflow

---

### Task 1: Add the provider-scoped runtime binding type

**Files:**
- Create: `crates/app/src/provider/runtime_binding.rs`
- Modify: `crates/app/src/provider/mod.rs`
- Test: `crates/app/src/provider/tests.rs`

**Step 1: Write the failing test**

Add or adapt provider tests so they call provider request/failover helpers with
`ProviderRuntimeBinding::direct()` and `ProviderRuntimeBinding::kernel(...)`
instead of raw optional kernel context.

**Step 2: Run test to verify it fails**

Run:
- `cargo test -p loongclaw-app provider_failover_audit_event_records_structured_payload -- --test-threads=1`
- `cargo test -p loongclaw-app provider_failover_metrics_record_even_without_kernel_context -- --test-threads=1`

Expected: FAIL because provider APIs still accept `Option<&KernelContext>`.

**Step 3: Write minimal implementation**

Create `ProviderRuntimeBinding` with explicit constructors/helpers and re-export
it from `provider/mod.rs`.

**Step 4: Run test to verify it passes**

Run the same commands and expect PASS.

### Task 2: Normalize provider request and failover seams

**Files:**
- Modify: `crates/app/src/provider/mod.rs`
- Modify: `crates/app/src/provider/request_failover_runtime.rs`
- Modify: `crates/app/src/provider/failover_telemetry_runtime.rs`
- Test: `crates/app/src/provider/tests.rs`
- Test: `crates/daemon/src/tests/import_cli.rs`

**Step 1: Write the failing test**

Adapt the request/failover tests that currently pass `None` or `Some(&ctx)` so
they use the explicit provider binding.

**Step 2: Run test to verify it fails**

Run:
- `cargo test -p loongclaw-app responses_completion_falls_back_to_chat_completions_for_compatible_endpoints -- --test-threads=1`
- `cargo test -p loongclaw-app responses_turn_falls_back_to_chat_completions_for_compatible_endpoints -- --test-threads=1`
- `cargo test -p loongclaw-app provider_failover_metrics_track_continue_path -- --test-threads=1`

Expected: FAIL until the provider request/failover signatures accept
`ProviderRuntimeBinding`.

**Step 3: Write minimal implementation**

Thread `ProviderRuntimeBinding` through the public provider request entrypoints,
failover orchestration, and failover telemetry helper. Only the audit leaf
should ask the binding for an optional kernel context. Update downstream test
callers that intentionally exercise direct provider execution.

**Step 4: Run test to verify it passes**

Run the same commands and expect PASS.

### Task 3: Translate at the conversation-to-provider boundary

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

If needed, adapt or add a conversation-runtime test that exercises both direct
and kernel-bound provider calls through the existing binding-based runtime path.

**Step 2: Run test to verify it fails**

Run:
- `cargo test -p loongclaw-app conversation_runtime_binding_direct_reports_no_kernel_context -- --test-threads=1`

Expected: FAIL only if a new translation helper/test is introduced; otherwise
this task is compile-time coverage plus targeted provider regression coverage.

**Step 3: Write minimal implementation**

Translate `ConversationRuntimeBinding` into `ProviderRuntimeBinding` at the
conversation runtime call sites instead of forwarding raw optional kernel
context.

**Step 4: Run test to verify it passes**

Run the same targeted command if a new test was added; otherwise rely on
provider and workspace verification.

### Task 4: Update security docs and finish verification

**Files:**
- Modify: `docs/SECURITY.md`
- Modify: `docs/plans/2026-03-15-provider-binding-normalization-design.md`
- Modify: `docs/plans/2026-03-15-provider-binding-normalization-implementation-plan.md`

**Step 1: Update docs**

Clarify that provider request/failover seams now use an explicit provider
runtime binding, while the remaining optional-kernel outer integration seam is
still limited to explicit boundary wrappers such as channel entrypoints.

**Step 2: Run targeted verification**

Run:
- `cargo test -p loongclaw-app responses_completion_falls_back_to_chat_completions_for_compatible_endpoints -- --test-threads=1`
- `cargo test -p loongclaw-app responses_turn_falls_back_to_chat_completions_for_compatible_endpoints -- --test-threads=1`
- `cargo test -p loongclaw-app provider_failover_audit_event_records_structured_payload -- --test-threads=1`
- `cargo test -p loongclaw-app provider_failover_metrics_record_even_without_kernel_context -- --test-threads=1`
- `cargo test -p loongclaw-app provider_failover_metrics_track_continue_path -- --test-threads=1`

Expected: PASS

**Step 3: Run full verification**

Run:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features -- --test-threads=1`

Expected: PASS

**Step 4: Commit**

```bash
git add docs/SECURITY.md \
        docs/plans/2026-03-15-provider-binding-normalization-design.md \
        docs/plans/2026-03-15-provider-binding-normalization-implementation-plan.md \
        crates/app/src/conversation/runtime.rs \
        crates/app/src/provider/failover_telemetry_runtime.rs \
        crates/app/src/provider/mod.rs \
        crates/app/src/provider/request_failover_runtime.rs \
        crates/app/src/provider/runtime_binding.rs \
        crates/app/src/provider/tests.rs \
        crates/daemon/src/tests/import_cli.rs
git commit -m "refactor: normalize provider runtime binding seams"
```
