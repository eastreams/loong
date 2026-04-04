# Channel Bridge Managed Discovery Remediation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add remediation guidance and ambiguity diagnostics to managed plugin bridge discovery for plugin-backed channel surfaces.

**Architecture:** Extend the app-owned discovery summary with setup-guidance and compatibility-selection facts, then let daemon turn those facts into surface-level doctor severity, next steps, and richer text/json output without changing operation-level bridge contract semantics.

**Tech Stack:** Rust app/kernel/daemon crates, serde-serializable inventory structs, doctor next-step policy, CLI integration tests.

---

## Task 1: Persist the approved design and analysis artifacts

**Files:**
- Create: `docs/plans/2026-04-01-channel-bridge-managed-discovery-remediation-design.md`
- Create: `docs/plans/2026-04-01-channel-bridge-managed-discovery-remediation-implementation-plan.md`
- Create (local/private, not in repo): `<lc-knowledge-base>/projects/loongclaw/analysis/2026/2026-04-01-channel-bridge-managed-discovery-remediation-analysis.md`

**Step 1: Save the public design**

Write the design document under `docs/plans/`.

**Step 2: Save the execution plan**

Write this implementation plan under `docs/plans/`.

**Step 3: Save the private analysis**

Archive the fuller reasoning, alternatives, and seam analysis in
`lc-knowledge-base`.

## Task 2: Add failing app tests for remediation and ambiguity facts

**Files:**
- Modify: `crates/app/src/channel/registry_plugin_bridge_tests.rs`
- Modify: `crates/app/src/channel/registry.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- discovered bridge plugins expose setup guidance from `PluginManifest.setup`
- discovery marks a surface as ambiguous when multiple ready-compatible managed
  plugins exist for the same channel
- ambiguity does not trigger when only incomplete or incompatible matches exist

**Step 2: Run focused tests to verify they fail**

Run:

`cargo test -p loongclaw-app channel::registry::plugin_bridge_tests:: -- --nocapture`

Expected: failures because discovery summaries do not expose setup guidance or a
typed compatibility-selection outcome yet.

## Task 3: Implement app-owned discovery guidance and ambiguity status

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/app/src/channel/registry_plugin_bridge.rs`

**Step 1: Add the public inventory types**

Add serde-serializable enums/fields for:

- managed discovery compatibility-selection status
- discovered plugin setup guidance facts

**Step 2: Populate setup guidance from manifest setup**

When building a discovered plugin summary, copy:

- `required_env_vars`
- `recommended_env_vars`
- `required_config_keys`
- `default_env_var`
- `docs_urls`
- `remediation`

**Step 3: Derive ambiguity from ready-compatible plugins**

Count ready-compatible plugins per surface and record whether the compatible
selection is absent, unique, or ambiguous.

## Task 4: Add failing daemon tests for ambiguity and next steps

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/tests/integration/mod.rs`

**Step 1: Write failing doctor tests**

Add tests that expect:

- managed discovery warns instead of passes when multiple ready-compatible
  plugins exist
- doctor next steps include concrete guidance for incomplete bridge manifests
- doctor next steps include de-ambiguation guidance for multiple compatible
  plugins

**Step 2: Write failing output tests**

Add tests that expect:

- channels text output includes compatibility-selection state
- channels JSON payload includes setup guidance and ambiguity status

**Step 3: Run focused tests to verify they fail**

Run:

- `cargo test -p loongclaw-daemon check_channel_surfaces -- --nocapture`
- `cargo test -p loongclaw-daemon build_doctor_next_steps -- --nocapture`
- `cargo test -p loongclaw-daemon build_channels_cli_json_payload_includes_managed_plugin_bridge_discovery -- --nocapture`

Expected: failures because daemon does not yet consume the new app-side facts.

## Task 5: Implement daemon-side ambiguity and remediation output

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/lib.rs`

**Step 1: Update managed discovery severity**

Treat ambiguous compatible discovery as a warning rather than a pass.

**Step 2: Add doctor next-step guidance**

Derive concrete next steps from:

- ambiguity state
- missing fields
- setup env/config requirements
- docs URLs
- manifest remediation text

**Step 3: Render the new facts**

Update text and JSON output so operators can see compatibility-selection status
and setup guidance.

## Task 6: Run verification

**Files:**
- Modify: none

**Step 1: Run focused tests**

Run:

- `cargo test -p loongclaw-app channel::registry::plugin_bridge_tests:: -- --nocapture`
- `cargo test -p loongclaw-daemon check_channel_surfaces -- --nocapture`
- `cargo test -p loongclaw-daemon build_doctor_next_steps -- --nocapture`
- `cargo test -p loongclaw-daemon build_channels_cli_json_payload_includes_managed_plugin_bridge_discovery -- --nocapture`

**Step 2: Run repository verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`
- `LOONGCLAW_ARCH_STRICT=true scripts/check_architecture_boundaries.sh`

**Step 3: Inspect the scoped diff**

Run:

- `git status --short`
- `git diff -- docs/plans/2026-04-01-channel-bridge-managed-discovery-remediation-design.md`
- `git diff -- docs/plans/2026-04-01-channel-bridge-managed-discovery-remediation-implementation-plan.md`
- `git diff -- crates/app/src/channel/registry.rs crates/app/src/channel/registry_plugin_bridge.rs crates/app/src/channel/registry_plugin_bridge_tests.rs crates/daemon/src/doctor_cli.rs crates/daemon/src/lib.rs crates/daemon/tests/integration/mod.rs`
