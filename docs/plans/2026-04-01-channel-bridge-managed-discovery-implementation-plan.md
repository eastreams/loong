# Channel Bridge Managed Discovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Expose managed plugin bridge discovery for plugin-backed channel surfaces through `ChannelSurface`, `loongclaw channels`, runtime snapshot, and `loongclaw doctor`.

**Architecture:** Reuse `kernel` scanning and translation to inspect manifests under `external_skills.install_root`, aggregate the results into an app-owned `ChannelSurface` managed-discovery summary, and let daemon consume that summary without changing existing operation-level bridge-contract checks.

**Tech Stack:** Rust app/kernel/daemon crates, serde-serializable inventory structs, channel doctor tests, CLI integration tests.

---

## Task 1: Persist the approved design and analysis artifacts

**Files:**
- Create: `docs/plans/2026-04-01-channel-bridge-managed-discovery-design.md`
- Create: `docs/plans/2026-04-01-channel-bridge-managed-discovery-implementation-plan.md`
- Create (local/private, not in repo): `<lc-knowledge-base>/projects/loongclaw/analysis/2026/2026-04-01-channel-bridge-managed-discovery-analysis.md`

**Step 1: Save the public design**

Write the design document under `docs/plans/`.

**Step 2: Save the execution plan**

Write this implementation plan under `docs/plans/`.

**Step 3: Save the private analysis**

Archive the fuller reasoning, alternatives, and seam analysis in
`lc-knowledge-base`.

## Task 2: Add failing app tests for managed discovery aggregation

**Files:**
- Modify: `crates/app/src/channel/registry_plugin_bridge_tests.rs`
- Modify: `crates/app/src/channel/registry.rs`

**Step 1: Write the failing tests**

Add tests that expect:

- `ChannelSurface` for `weixin` exposes managed discovery when a compatible
  plugin manifest exists under `external_skills.install_root`
- incomplete bridge metadata is reported as incomplete rather than compatible
- unsupported setup surface is reported as incompatible
- missing install root is reported as discovery unavailable

**Step 2: Run focused tests to verify they fail**

Run:

`cargo test -p loongclaw-app channel::registry_plugin_bridge_tests:: -- --nocapture`

Expected: failures because the inventory model does not expose managed discovery
yet.

## Task 3: Implement app-owned managed discovery summaries

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/app/src/channel/registry_plugin_bridge.rs`

**Step 1: Add the public inventory types**

Add serde-serializable discovery structs and enums to represent:

- discovery availability / scan status
- discovered plugin status
- matched plugin detail
- per-surface managed discovery summary

**Step 2: Scan the managed install root**

In the plugin-bridge helper module:

- resolve `external_skills.install_root`
- scan it with `kernel::PluginScanner`
- translate descriptors with `kernel::PluginTranslator`
- group matched manifests by plugin-backed channel surface

**Step 3: Build `ChannelSurface` summaries**

Attach the managed discovery summary during `channel_inventory_with_now`
assembly without changing `ChannelCatalogEntry`.

## Task 4: Add failing daemon tests for doctor and channels output

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/tests/integration/mod.rs`

**Step 1: Write failing doctor tests**

Add tests that expect:

- existing bridge contract checks keep their current pass/fail semantics
- a new surface-level managed discovery check is emitted for configured
  plugin-backed surfaces
- incomplete managed plugins warn instead of being reported as compatible

**Step 2: Write failing channel-output tests**

Add tests that expect:

- `channel_surfaces[*].plugin_bridge_discovery` appears in JSON output
- text rendering prints the managed discovery summary for plugin-backed surfaces

**Step 3: Run focused tests to verify they fail**

Run:

- `cargo test -p loongclaw-daemon build_channel_surface_checks_reports_plugin_bridge_contract_status_for_configured_surface -- --nocapture`
- `cargo test -p loongclaw-daemon build_channels_cli_json_payload_includes_plugin_bridge_contracts -- --nocapture`

Expected: failures because daemon does not consume the new summary yet.

## Task 5: Implement daemon consumption of managed discovery

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/lib.rs`

**Step 1: Switch doctor channel collection to inventory**

Update `check_channel_surfaces` so doctor can read both configured snapshots and
surface-level managed discovery from `ChannelInventory`.

**Step 2: Add surface-level managed discovery checks**

Add a new helper that maps the app summary to doctor checks without changing
the existing operation-level bridge checks.

**Step 3: Render the summary in channel output**

Update the channels text renderer to print managed discovery summary lines and
matched plugin details.

## Task 6: Run verification

**Files:**
- Modify: none

**Step 1: Run focused app and daemon tests**

Run:

- `cargo test -p loongclaw-app channel::registry_plugin_bridge_tests:: -- --nocapture`
- `cargo test -p loongclaw-daemon build_channel_surface_checks -- --nocapture`
- `cargo test -p loongclaw-daemon build_channels_cli_json_payload_includes_plugin_bridge_contracts -- --nocapture`

**Step 2: Run repository verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

**Step 3: Inspect the scoped diff**

Run:

- `git status --short`
- `git diff -- docs/plans/2026-04-01-channel-bridge-managed-discovery-design.md`
- `git diff -- docs/plans/2026-04-01-channel-bridge-managed-discovery-implementation-plan.md`
- `git diff -- crates/app/src/channel/registry.rs crates/app/src/channel/registry_plugin_bridge.rs crates/app/src/channel/registry_plugin_bridge_tests.rs crates/daemon/src/doctor_cli.rs crates/daemon/src/lib.rs crates/daemon/tests/integration/mod.rs`
