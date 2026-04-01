# Weixin and QQBot Bridge Diagnostics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add truthful doctor coverage and onboarding diagnostics for the plugin-backed `weixin`, `qqbot`, and `onebot` bridge surfaces.

**Architecture:** Extend the shared channel registry with a bridge-specific doctor trigger, wire the daemon doctor path to interpret plugin-owned bridge snapshots correctly, and update onboarding metadata to point operators at `loongclaw doctor`. Keep all semantics attached to registry descriptors and shared inventory data rather than daemon-local channel special cases.

**Tech Stack:** Rust channel registry metadata, daemon doctor CLI logic, Rust unit tests, Markdown design and analysis docs.

---

### Task 1: Save the design and analysis artifacts

**Files:**
- Create: `docs/plans/2026-04-01-weixin-qqbot-bridge-diagnostics-design.md`
- Create: `/Users/chum/lc-knowledge-base/projects/loongclaw/analysis/2026/2026-04-01-weixin-qqbot-bridge-diagnostics-analysis.md`

**Step 1: Persist the public design**

Write the repo-facing design document that explains the root cause and chosen
registry-first fix.

**Step 2: Persist the private analysis**

Archive the broader reasoning, rejected approaches, and external reference notes
under `lc-knowledge-base`.

### Task 2: Add failing registry and doctor tests

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

**Step 1: Write failing registry tests**

Add tests that expect:

- plugin-backed surfaces to use `loongclaw doctor` as `status_command`
- plugin-backed operations to resolve bridge-specific doctor trigger metadata

**Step 2: Write failing doctor tests**

Add tests that expect:

- configured bridge-backed send and serve surfaces to pass bridge contract
  checks instead of failing on `Unsupported`
- misconfigured bridge-backed surfaces to still fail
- uncompiled bridge-backed surfaces to still fail

**Step 3: Run focused tests to verify they fail**

Run:

- `cargo test -p loongclaw-app channel::registry::tests::resolve_channel_catalog_entry_exposes_onboarding_contracts`
- `cargo test -p loongclaw-app channel::registry::tests::resolve_channel_doctor_operation_spec_uses_registry_metadata`
- `cargo test -p loongclaw-daemon doctor_cli::tests::build_channel_surface_checks_reports_plugin_bridge_contract_status_for_configured_surface`

Expected: failures because the new trigger and onboarding metadata do not exist
yet.

### Task 3: Add the bridge-specific registry metadata

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/app/src/channel/registry_bridge.rs`

**Step 1: Extend the doctor trigger enum**

Add a new trigger variant for plugin-backed bridge diagnostics.

**Step 2: Define bridge doctor specs**

Attach bridge-specific doctor check specs to the `send` and `serve` operation
descriptors for:

- `weixin`
- `qqbot`
- `onebot`

**Step 3: Update onboarding metadata**

Switch bridge-backed onboarding `status_command` to `loongclaw doctor` while
leaving `repair_command` unset.

### Task 4: Implement doctor-side bridge health interpretation

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`

**Step 1: Add a bridge-health builder**

Implement a focused helper that maps a bridge-backed snapshot and operation to a
truthful `DoctorCheck`.

**Step 2: Keep generic health logic untouched**

Continue using the existing `doctor_check_level_for_health` helper for the
generic trigger paths.

**Step 3: Route the new trigger**

Teach `build_channel_operation_doctor_check` to use the new bridge helper only
for the new trigger variant.

### Task 5: Re-run focused tests and tighten names if needed

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/app/src/channel/registry_bridge.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

**Step 1: Run the focused registry tests**

Run:

- `cargo test -p loongclaw-app channel::registry::tests::resolve_channel_catalog_entry_exposes_onboarding_contracts`
- `cargo test -p loongclaw-app channel::registry::tests::resolve_channel_doctor_operation_spec_uses_registry_metadata`

Expected: pass.

**Step 2: Run the focused doctor tests**

Run:

- `cargo test -p loongclaw-daemon doctor_cli::tests::build_channel_surface_checks_reports_plugin_bridge_contract_status_for_configured_surface`
- `cargo test -p loongclaw-daemon doctor_cli::tests::build_channel_surface_checks_fails_plugin_bridge_contract_when_surface_is_uncompiled`

Expected: pass.

**Step 3: Refine wording only after green**

If test output shows awkward operator-facing names or details, make the smallest
text-only cleanup and keep the tests green.

### Task 6: Run full verification

**Files:**
- Modify: none

**Step 1: Run targeted crate tests**

Run:

- `cargo test -p loongclaw-app --locked`
- `cargo test -p loongclaw-daemon --locked doctor_cli::tests::`

**Step 2: Run repository verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`

**Step 3: Inspect the diff**

Run:

- `git status --short`
- `git diff -- crates/app/src/channel/registry.rs crates/app/src/channel/registry_bridge.rs crates/daemon/src/doctor_cli.rs docs/plans/2026-04-01-weixin-qqbot-bridge-diagnostics-design.md docs/plans/2026-04-01-weixin-qqbot-bridge-diagnostics-implementation-plan.md`

Plan complete and saved to `docs/plans/2026-04-01-weixin-qqbot-bridge-diagnostics-implementation-plan.md`.
