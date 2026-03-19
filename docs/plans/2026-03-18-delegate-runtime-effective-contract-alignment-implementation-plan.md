# Delegate Runtime Effective Contract Alignment Implementation Plan

**Goal:** Align delegate child prompt visibility with the same effective runtime contract that child
tool execution enforces.

**Architecture:** Move child prompt-summary rendering from raw `ToolRuntimeNarrowing` inputs to
effective `ToolRuntimeConfig::narrowed(...)` outputs while keeping the existing child-only system
prompt injection seam.

**Tech Stack:** Rust, conversation runtime prompt assembly, runtime-config narrowing logic, existing
delegate child session fixtures, workspace cargo verification.

---

## Task 1: Add failing effective-contract tests

**Files:**
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Add a failing formatter test for stricter base runtime**

Create a unit test proving prompt formatting uses effective values when the base runtime is stricter
than the child narrowing input.

Example case:
- base `allow_private_hosts = false`
- child narrowing `allow_private_hosts = Some(true)`
- expected summary must not say `allowed`

**Step 2: Add a failing formatter test for empty allowlist intersection**

Create a test where:
- base allowed domains = `api.example.com`
- child narrowing allowed domains = `docs.example.com`
- expected summary says the effective allowed-domain set is empty

**Step 3: Add a failing conversation/runtime integration test**

Persist a delegate child session with child narrowing, configure the base runtime to be stricter
than the child request, build context, and assert the child prompt shows the effective contract.

**Step 4: Run the new focused tests and verify RED**

Run the precise runtime-config and conversation tests and confirm they fail for the current raw
formatter path.

## Task 2: Implement effective prompt-summary rendering

**Files:**
- Modify: `crates/app/src/tools/runtime_config.rs`

**Step 1: Replace raw summary formatting API**

Move prompt-summary rendering onto `ToolRuntimeConfig` so the formatter has access to base runtime
state before rendering.

**Step 2: Compute effective values through `narrowed(...)`**

Use the same `ToolRuntimeConfig::narrowed(...)` path that execution already trusts.

**Step 3: Render fail-closed intersections explicitly**

When child-requested `allowed_domains` collapse to an empty effective set, render a stable
explicit line instead of omitting the field.

**Step 4: Preserve deterministic output**

Keep stable ordering and sparse field emission so prompt behavior and tests remain predictable.

## Task 3: Wire conversation runtime to the new effective summary API

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`

**Step 1: Pass base runtime config into child summary derivation**

Use `ToolRuntimeConfig::from_loongclaw_config(config, None)` as the base runtime source for prompt
summary derivation.

**Step 2: Keep the current merge behavior**

Preserve existing `system_prompt_addition` merging and child-only gating.

**Step 3: Avoid widening prompt scope**

Only replace the summary source of truth. Do not add new prompt channels or rewrite tool-view
projection.

## Task 4: Verify locally and prepare delivery

**Files:**
- Modify: current branch / PR artifacts as needed

**Step 1: Run focused tests**

Run the new RED/GREEN tests first.

**Step 2: Run repository verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`
- `LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh`
- `./scripts/check_dep_graph.sh`
- `diff -u CLAUDE.md AGENTS.md`

**Step 3: Prepare delivery artifacts**

- update the active branch or PR with the verified fix
- keep the tracking issue link in the delivery artifact that lands the change
- reflect the real verification state, including any remaining prerequisites or known gaps
