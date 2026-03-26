# Autonomy Policy Kernel Implementation Plan

> Execution note: implement this plan in follow-up slices after issue `#596`
> is accepted.

**Goal:** Keep `product mode` as the operator-facing profile surface, while
moving the internal runtime control plane to an autonomy-policy kernel that can
later support learning and governed evolution.

**Architecture:** `product mode` resolves into an `AutonomyPolicySnapshot`.
Discovery-first remains the lower-layer selection substrate. The autonomy-policy
kernel decides whether capability-acquisition actions are allowed, approval
bound, or denied. Future learning ranks among allowed actions. Future evolution
proposes and validates policy or strategy changes outside the live turn path.

**Tech Stack:** Rust, existing `loongclaw-app` tool and conversation runtime,
channel SDK metadata, runtime config, approval and session persistence,
docs-first delivery, GitHub issue-first workflow

---

## Execution Tasks

Verification note: keep test prefixes slice-specific so every command can target
one slice with a single positional filter.

### Task 1: Land the docs contract

**Files:**
- Modify: `docs/plans/2026-03-26-product-mode-capability-acquisition-design.md`
- Modify: `docs/plans/2026-03-26-product-mode-capability-acquisition-implementation-plan.md`
- Create: `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`
- Create: `docs/plans/2026-03-26-autonomy-policy-kernel-implementation-plan.md`

**Step 1: Write the artifacts**

- clarify that `product mode` remains the product-facing profile surface
- define the internal autonomy-policy kernel
- define the decision contract and hard-vs-learnable boundary
- define the governed evolution plane and its relationship to issue `#455`
- document what Hermes Agent and HyperAgents suggest, and what should remain
  outside LoongClaw's live turn path

**Step 2: Verify the artifacts exist**

Run:

```bash
test -f docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md
test -f docs/plans/2026-03-26-autonomy-policy-kernel-implementation-plan.md
git diff --check
```

Expected: success

### Task 2: Introduce product profile and autonomy-policy types

**Files:**
- Modify: `crates/app/src/config/*.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/runtime_env.rs`
- Test: `crates/app/src/config/*`
- Test: `crates/app/src/tools/runtime_config.rs`

**Step 1: Write the failing tests**

Add tests with an `autonomy_profile_runtime_config_` prefix that prove:

- product-facing profiles have a stable enum surface
- runtime config resolves a default profile deterministically
- a profile compiles into a policy snapshot
- invalid profile or policy values fail closed

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app autonomy_profile_runtime_config_ -- --test-threads=1
```

Expected: FAIL because the runtime does not yet expose profile-to-policy
compilation.

**Step 3: Write minimal implementation**

Introduce types such as:

- `AutonomyProfile`
- `AutonomyPolicySnapshot`
- `AutonomyBudgetPolicy`

Keep `product mode` as the product-facing vocabulary if that is already exposed
elsewhere, but compile it into explicit policy fields instead of encoding every
rule directly on the mode enum.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 3: Add capability-action classification to the tool catalog

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Test: `crates/app/src/tools/catalog.rs`

**Step 1: Write the failing tests**

Add tests with an `autonomy_capability_action_` prefix that prove:

- capability action classes exist independently of governance profile
- known mutation tools classify correctly
- new tools can be added without scattering raw-name checks across the runtime

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app autonomy_capability_action_ -- --test-threads=1
```

Expected: FAIL because the catalog does not yet expose typed action classes.

**Step 3: Write minimal implementation**

Add a stable action-class enum to the tool catalog and expose it through the
descriptor or a derived catalog view.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 4: Add the autonomy decision engine

**Files:**
- Create: `crates/app/src/conversation/autonomy_policy.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests with an `autonomy_policy_decision_` prefix that prove:

- the decision engine consumes profile, policy snapshot, action class,
  governance profile, binding facts, channel facts, and budget facts
- the engine returns only `allow`, `approval_required`, or `deny`
- the engine keeps hard constraints deterministic

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app autonomy_policy_decision_ -- --test-threads=1
```

Expected: FAIL because the decision engine does not yet exist.

**Step 3: Write minimal implementation**

Implement:

- `PolicyDecisionInput`
- `PolicyDecisionOutcome`
- deterministic evaluation logic

Keep learning and ranking out of this layer.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 5: Bind the autonomy kernel to runtime binding and channel SDK support

**Files:**
- Modify: `crates/app/src/conversation/runtime_binding.rs`
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/channel/sdk.rs`
- Test: `crates/app/src/conversation/tests.rs`
- Test: `crates/app/src/channel/sdk.rs`

**Step 1: Write the failing tests**

Add tests with an `autonomy_policy_surface_` prefix that prove:

- weak bindings fail closed for mutation paths
- explanation-only deny outcomes may still execute on weak bindings
- channel SDK surfaces can declare whether kernel-backed mutation and approval
  round-trips are supported
- unsupported profile and surface combinations are rejected early

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app autonomy_policy_surface_ -- --test-threads=1
```

Expected: FAIL because the runtime and SDK are not yet autonomy-policy-aware.

**Step 3: Write minimal implementation**

Extend SDK metadata with the smallest set of fields needed to express:

- supported product profiles
- kernel-backed mutation support
- approval round-trip support
- session override support

Keep channel SDK metadata declarative. Do not move autonomy heuristics into
channel-specific code.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 6: Add explicit policy telemetry for later learning work

**Files:**
- Modify: `crates/app/src/session/repository.rs`
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Modify: `crates/app/src/tools/approval.rs`
- Test: `crates/app/src/session/repository.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests with an `autonomy_policy_telemetry_` prefix that prove:

- decision outcomes persist explicit reason codes
- approval-required outcomes persist their policy source
- blocked explanations preserve enough structure for later replay and analysis

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app autonomy_policy_telemetry_ -- --test-threads=1
```

Expected: FAIL because the runtime does not yet persist autonomy-policy outcome
artifacts explicitly.

**Step 3: Write minimal implementation**

Add structured persistence for:

- resolved profile
- policy decision outcome
- reason code
- relevant action class

This is the minimum foundation for later ranking or replay work.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 7: Add a governed evolution lane outside the live turn path

**Files:**
- Create: future follow-up slice after Tasks 2-6 land

**Step 1: Design first**

Do not start by mutating the live autonomy policy online.

The first governed evolution slice should define:

- candidate policy artifact shape
- replay or shadow evaluation contract
- promotion decision rules
- rollback path

**Step 2: Implement later**

Only after earlier tasks land, add bounded experiment and promotion support.

## Recommended Slice Order

1. docs refinement
2. product profile and policy snapshot types
3. capability-action classification
4. decision engine
5. binding and channel SDK support
6. policy telemetry
7. governed evolution plane

## Full Verification

After runtime slices land, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
```

Expected: PASS

For the current docs-only slice, run:

```bash
git diff --check
```

Expected: PASS
