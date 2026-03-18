# Retire the Implicit Noop Audit Default Implementation Plan

Goal: make the default kernel constructor safe by replacing the implicit `NoopAuditSink` default
with in-memory audit, while preserving an explicit no-audit escape hatch and keeping the change
additive.

Architecture: keep `AuditSink` injection intact. Fix the bug at the constructor seam instead of
reworking bootstrap layers. `LoongClawKernel::new()` becomes the safe convenience path,
`with_runtime(...)` remains the fully explicit path, and a narrowly named no-audit helper makes
intent grep-able.

Tech stack: Rust, kernel/app tests, cargo fmt, cargo clippy, workspace tests, repo check scripts

---

### Task 1: Lock the constructor design

**Files:**
- Create: `docs/plans/2026-03-18-retire-noop-audit-default-design.md`
- Create: `docs/plans/2026-03-18-retire-noop-audit-default-implementation-plan.md`

**Step 1: Confirm the root-cause seam**

Run: `rg -n "NoopAuditSink|LoongClawKernel::new\\(" crates/kernel crates/app docs`
Expected: the default constructor seam and current callsites are enumerated.

**Step 2: Confirm the plan files exist**

Run: `ls docs/plans/2026-03-18-retire-noop-audit-default-design.md docs/plans/2026-03-18-retire-noop-audit-default-implementation-plan.md`
Expected: both files exist.

### Task 2: Add regression coverage before the implementation

**Files:**
- Modify: `crates/kernel/src/tests.rs`
- Modify: `crates/kernel/tests/kernel_integration.rs`

**Step 1: Add a failing default-constructor audit regression**

Cover the safe default lane with a helper that makes the default in-memory audit observable, then
assert token issuance records an audit event.

**Step 2: Add a failing explicit no-audit regression**

Add a test proving the intentionally named no-audit constructor still allows side-effect-free
fixtures when callers opt into it explicitly.

**Step 3: Run the targeted kernel tests and confirm RED**

Run: `cargo test -p loongclaw-kernel default_constructor_audit explicit_no_audit -- --test-threads=1`
Expected: FAIL before the constructor helpers exist.

### Task 3: Implement the additive constructor changes

**Files:**
- Modify: `crates/kernel/src/kernel.rs`
- Modify: `crates/kernel/src/lib.rs`
- Modify: `crates/kernel/tests/kernel_integration.rs`
- Modify: `crates/app/src/memory/tests.rs`

**Step 1: Add explicit constructor helpers**

Introduce:

1. a safe in-memory constructor/helper for tests and default convenience
2. a deliberately named no-audit constructor

Keep `with_runtime(...)` unchanged.

**Step 2: Retire the implicit noop default**

Switch `LoongClawKernel::new()` to the safe default helper rather than `NoopAuditSink`.

**Step 3: Migrate repository-owned ambiguous callsites**

Replace local `LoongClawKernel::new()` uses with explicit helper calls where doing so improves
audit intent without large churn.

**Step 4: Re-export any new constructor-supporting types if needed**

Only expose new items from `crates/kernel/src/lib.rs` if the implementation introduces reusable
surface beyond existing exports.

### Task 4: Reconcile docs

**Files:**
- Modify: `docs/RELIABILITY.md`
- Modify: `docs/design-docs/core-beliefs.md` if wording needs precision

**Step 1: Update reliability wording**

Document that the default constructor now records to in-memory audit, while any silent-drop path
must be explicit.

**Step 2: Review doc scope**

Run: `git diff -- docs/RELIABILITY.md docs/design-docs/core-beliefs.md`
Expected: only constructor-semantics wording changes are present.

### Task 5: Run full verification and prepare delivery

**Files:**
- Modify: `crates/kernel/src/kernel.rs`
- Modify: `crates/kernel/src/tests.rs`
- Modify: `crates/kernel/tests/kernel_integration.rs`
- Modify: `crates/app/src/memory/tests.rs`
- Modify: `docs/RELIABILITY.md`
- Modify: `docs/design-docs/core-beliefs.md` if needed
- Create: `docs/plans/2026-03-18-retire-noop-audit-default-design.md`
- Create: `docs/plans/2026-03-18-retire-noop-audit-default-implementation-plan.md`

**Step 1: Run format and lint**

Run: `cargo fmt --all -- --check`
Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS.

**Step 2: Run workspace tests**

Run: `cargo test --workspace -- --test-threads=1`
Run: `cargo test --workspace --all-features -- --test-threads=1`
Expected: PASS.

**Step 3: Run repo guardrails**

Run: `./scripts/check_architecture_boundaries.sh`
Run: `./scripts/check_dep_graph.sh`
Run: `./scripts/check-docs.sh`
Expected: PASS.

**Step 4: Review the final scoped diff**

Run: `git diff -- crates/kernel/src/kernel.rs crates/kernel/src/tests.rs crates/kernel/tests/kernel_integration.rs crates/app/src/memory/tests.rs docs/RELIABILITY.md docs/design-docs/core-beliefs.md docs/plans/2026-03-18-retire-noop-audit-default-design.md docs/plans/2026-03-18-retire-noop-audit-default-implementation-plan.md`
Expected: only the constructor-audit slice is present.
