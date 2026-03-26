# Product Mode Capability Acquisition Implementation Plan

> Execution note: implement this plan in follow-up slices after the contract in
> issue `#581` is accepted.

**Goal:** Add an explicit product-mode contract for capability acquisition and
bounded autonomous expansion without replacing the existing discovery-first
runtime.

**Architecture:** Keep `tool.search` and `tool.invoke` as the discovery-first
substrate, then layer a mode-aware capability-acquisition policy above them.
Mode policy decides whether the runtime may fetch/install/load/switch
capability, whether approval is required, whether kernel binding is mandatory,
and how blocked actions are explained.

**Tech Stack:** Rust, existing `loongclaw-app` conversation/tool/session
surfaces, config/runtime wiring, channel descriptor metadata, cargo test, cargo
clippy, GitHub issue-first workflow

---

## Execution Tasks

Verification note: give each new slice-specific test a shared prefix so every
`cargo test` command can target the slice with one positional filter instead of
an invalid multi-filter invocation.

### Task 1: Land the design and roadmap contract

**Files:**
- Create: `docs/plans/2026-03-26-product-mode-capability-acquisition-design.md`
- Create: `docs/plans/2026-03-26-product-mode-capability-acquisition-implementation-plan.md`

**Step 1: Write the artifacts**

- define `discovery_only`, `guided_acquisition`, and `bounded_autonomous`
- define capability action classes
- define the turn-level product-mode state machine
- define approval, kernel-binding, and channel-descriptor constraints

**Step 2: Verify the artifacts exist**

Run:

```bash
test -f docs/plans/2026-03-26-product-mode-capability-acquisition-design.md
test -f docs/plans/2026-03-26-product-mode-capability-acquisition-implementation-plan.md
```

Expected: success

### Task 2: Add the product-mode type and runtime config surface

**Files:**
- Modify: `crates/app/src/config/*.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/runtime_env.rs`
- Test: `crates/app/src/config/*`
- Test: `crates/app/src/tools/runtime_config.rs`

**Step 1: Write the failing tests**

Add tests with a `product_mode_runtime_config_` prefix that prove:

- product mode has a stable enum surface
- runtime config can resolve a default product mode
- bounded autonomy budgets are parsed deterministically
- invalid or unknown product-mode values fail closed

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app product_mode_runtime_config_ -- --test-threads=1
```

Expected: FAIL because no product-mode config surface exists yet.

**Step 3: Write minimal implementation**

Add a new mode contract such as:

- `discovery_only`
- `guided_acquisition`
- `bounded_autonomous`

Add minimal runtime policy fields such as:

- default mode
- max acquisition actions per turn
- max acquisition rounds per turn
- max provider switches per turn
- allow autonomous provider switch
- require kernel binding for non-discovery modes
- fail_closed_on_unsupported_surface

Also define deterministic mode resolution precedence:

- explicit session override
- channel surface default
- global runtime default

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 3: Add capability action classification to the tool catalog

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Test: `crates/app/src/tools/catalog.rs`

**Step 1: Write the failing tests**

Add tests with a `capability_action_class_` prefix that prove:

- `external_skills.fetch` is classified as `capability_fetch`
- `external_skills.install` is classified as `capability_install`
- `external_skills.invoke` is classified as `capability_load`
- `provider.switch` is classified as `runtime_switch`
- `delegate` and `delegate_async` are classified as `topology_expand`

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app capability_action_class_ -- --test-threads=1
```

Expected: FAIL because the catalog does not yet expose this classification.

**Step 3: Write minimal implementation**

Add a stable action-class enum to the tool catalog layer. Keep this separate
from governance profile so product mode can reason over action families instead
of raw tool names.

Store the action class on the tool descriptor or a derived catalog view so the
runtime does not grow scattered tool-name string matching.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 4: Add the mode policy evaluator

**Files:**
- Create: `crates/app/src/conversation/product_mode.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests with a `product_mode_policy_` prefix that prove:

- `discovery_only` rejects capability-acquisition actions
- `guided_acquisition` emits approval-required outcomes for acquisition actions
- `bounded_autonomous` allows only configured acquisition classes
- topology mutation remains blocked or approval-bound even in bounded autonomy

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app product_mode_policy_ -- --test-threads=1
```

Expected: FAIL because the policy evaluator does not exist.

**Step 3: Write minimal implementation**

Add a deterministic evaluator that consumes:

- current product mode
- capability action class
- governance profile
- runtime binding strength
- current turn budget state

and returns:

- `allow`
- `approval_required`
- `blocked(reason_code)`

Keep the evaluator layered. It should not replace source-policy checks in
`external_skills` or provider-switch validation. It should decide whether the
action class is even allowed to proceed to those lower layers.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 5: Bind product mode to conversation runtime and fail closed on weak binding

**Files:**
- Modify: `crates/app/src/conversation/runtime_binding.rs`
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests with a `product_mode_binding_` prefix that prove:

- `discovery_only` remains valid on `ConversationRuntimeBinding::Direct`
- `guided_acquisition` fails closed when the runtime cannot support approval or
  durable mutation semantics
- `bounded_autonomous` fails closed when binding is direct

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app product_mode_binding_ -- --test-threads=1
```

Expected: FAIL because binding is currently mode-agnostic.

**Step 3: Write minimal implementation**

Do not remove direct fallback globally. Instead:

- keep `Direct` valid for `discovery_only`
- require kernel-bound execution for approval-pending and acquisition paths
- allow weak-binding execution only for deterministic explanation-only blocked
  outcomes that emit no approval request and perform no mutation
- surface explicit blocked reason codes when the binding is too weak

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 6: Add mode-aware approval reasons and blocked explanations

**Files:**
- Modify: `crates/app/src/tools/approval.rs`
- Modify: `crates/app/src/session/repository.rs`
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Test: `crates/app/src/tools/approval.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing tests**

Add tests with a `product_mode_outcome_` prefix that prove:

- capability-acquisition approvals carry mode-specific reason codes
- blocked actions produce operator-visible reason codes
- provider follow-up text explains mode blocks instead of surfacing only
  "tool not found" style errors

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app product_mode_outcome_ -- --test-threads=1
```

Expected: FAIL because approval and follow-up flows are not yet mode-aware.

**Step 3: Write minimal implementation**

Add reason codes such as:

- `product_mode_capability_fetch_requires_approval`
- `product_mode_capability_install_requires_approval`
- `product_mode_provider_switch_requires_approval`
- `product_mode_disallows_capability_acquisition`
- `product_mode_kernel_binding_missing`
- `product_mode_autonomy_budget_exceeded`
- `product_mode_unsupported_by_channel_surface`
- `product_mode_approval_roundtrip_unavailable`

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 7: Add channel descriptor product-mode surfaces

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/channel/registry.rs`
- Test: `crates/app/src/config/channels.rs`
- Test: `crates/app/src/channel/registry.rs`

**Step 1: Write the failing tests**

Add tests with a `channel_product_mode_` prefix that prove:

- channel integrations expose allowed product modes
- channels can declare whether kernel-bound execution is guaranteed
- channels can declare whether approval round-trips are supported
- channels can declare whether session overrides are allowed
- unsupported product modes are rejected early

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app channel_product_mode_ -- --test-threads=1
```

Expected: FAIL because channel descriptors are not yet mode-aware.

**Step 3: Write minimal implementation**

Extend channel descriptors with fields such as:

- default product mode
- supported product modes
- supports approval resolution
- guarantees kernel-bound execution
- allows session-level product-mode override

Keep the integration metadata channel-agnostic.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 8: Optionally extend plan IR after the product contract lands

**Files:**
- Modify: `crates/app/src/conversation/plan_ir.rs`
- Modify: `crates/app/src/conversation/plan_executor.rs`
- Test: `crates/app/src/conversation/plan_*`

**Step 1: Write the failing tests**

Add tests with a `plan_ir_capability_` prefix that prove:

- plan graphs can represent `AcquireCapability`
- plan graphs can represent `AwaitApproval`
- plan graphs can represent `RefreshDiscovery`

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-app plan_ir_capability_ -- --test-threads=1
```

Expected: FAIL because plan IR currently models only tool/transform/verify/respond.

**Step 3: Write minimal implementation**

Only after earlier tasks land, extend plan IR with capability-acquisition nodes.
Do not start here.

**Step 4: Run test to verify it passes**

Run the same command and expect PASS.

### Task 9: Full verification

**Step 1: Run targeted verification**

Run:

```bash
cargo test -p loongclaw-app product_mode_runtime_config_ -- --test-threads=1
cargo test -p loongclaw-app capability_action_class_ -- --test-threads=1
cargo test -p loongclaw-app product_mode_policy_ -- --test-threads=1
cargo test -p loongclaw-app product_mode_binding_ -- --test-threads=1
cargo test -p loongclaw-app product_mode_outcome_ -- --test-threads=1
cargo test -p loongclaw-app channel_product_mode_ -- --test-threads=1
```

Expected: PASS

If Task 8 landed, also run:

```bash
cargo test -p loongclaw-app plan_ir_capability_ -- --test-threads=1
```

**Step 2: Run full verification**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
```

Expected: PASS

## Recommended Slice Order

1. docs contract
2. product-mode config type
3. capability action classification
4. mode policy evaluator
5. binding fail-closed enforcement
6. approval and blocked explanation integration
7. channel integration metadata
8. optional plan-IR follow-up

This order keeps the first implementation slice small and avoids mixing runtime
mutation policy with plan-graph expansion before the core product contract is
stable.
