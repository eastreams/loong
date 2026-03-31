# Weixin and QQBot Channel Bridge Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add truthful plugin-backed channel catalog support for `weixin`, `qqbot`, and `onebot`, plus the docs and plugin-contract guidance needed for bridge-first integration.

**Architecture:** Extend the existing channel registry model with a plugin-backed implementation status and onboarding strategy, then add new catalog entries for the bridge-first surfaces. Reuse the existing plugin manifest seam rather than inventing a second plugin contract, and document stable setup and target conventions for future adapters and plugins.

**Tech Stack:** Rust channel registry metadata and tests, Markdown product docs, plugin manifest contract docs.

---

### Task 1: Save the approved design and analysis artifacts

**Files:**
- Create: `docs/plans/2026-03-31-weixin-qqbot-channel-bridge-support-design.md`
- Create: `/Users/chum/lc-knowledge-base/projects/loongclaw/analysis/2026/2026-03-31-weixin-qqbot-channel-bridge-support-analysis.md`

**Step 1: Persist the public design**

Write the approved LoongClaw-facing design doc under `docs/plans/`.

**Step 2: Persist the private analysis**

Archive the broader reasoning and ecosystem references in
`lc-knowledge-base`.

### Task 2: Add failing registry tests for the new support tier

**Files:**
- Modify: `crates/app/src/channel/registry.rs`

**Step 1: Write failing tests**

Add tests that expect:

- `ChannelCatalogImplementationStatus` to expose `plugin_backed`
- `ChannelOnboardingStrategy` to expose `plugin_bridge`
- the catalog to include `weixin`, `qqbot`, and `onebot`
- alias normalization for `wechat`, `qq`, and `onebot-v11`
- the new surfaces to report the right target kinds and onboarding strategy

**Step 2: Run focused tests to verify they fail**

Run:

`cargo test -p loongclaw-app channel::registry::tests::normalize_channel_catalog_id_maps_runtime_and_stub_aliases`

and the new targeted registry tests.

Expected: failure because the new enums and catalog entries do not exist yet.

### Task 3: Implement the plugin-backed registry semantics

**Files:**
- Modify: `crates/app/src/channel/registry.rs`

**Step 1: Add the new status and onboarding semantics**

Extend:

- `ChannelCapability`
- `ChannelOnboardingStrategy`
- `ChannelCatalogImplementationStatus`

with plugin-backed variants and string renderers.

**Step 2: Add the new bridge-first surfaces**

Implement channel metadata for:

- `weixin`
- `qqbot`
- `onebot`

including:

- requirement constants
- send/serve operation descriptors
- onboarding descriptors
- registry entries and selection order

**Step 3: Keep operations honest**

Mark the new send/serve operations as `stub` while giving the surfaces the
overall implementation status `plugin_backed`.

### Task 4: Make the catalog tests pass

**Files:**
- Modify: `crates/app/src/channel/registry.rs`

**Step 1: Update affected expectations**

Adjust existing list-order and catalog-only expectations so the new surfaces are
included in the correct sorted positions.

**Step 2: Re-run focused tests**

Run:

- `cargo test -p loongclaw-app channel::registry::tests::normalize_channel_catalog_id_maps_runtime_and_stub_aliases`
- `cargo test -p loongclaw-app channel::registry::tests::catalog_only_channel_entries_include_stub_surfaces_for_default_config`
- `cargo test -p loongclaw-app channel::registry::tests::channel_inventory_combines_runtime_and_catalog_surfaces`

Expected: pass.

### Task 5: Document the product-facing support contract

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/product-specs/channel-setup.md`

**Step 1: Update the public channel catalog copy**

Add `Weixin`, `QQBot`, and `OneBot` to the broader catalog documentation.

**Step 2: Document the new support tier**

Describe plugin-backed surfaces as a distinct layer between fully shipped native
surfaces and pure future stubs.

**Step 3: Document bridge-first setup**

Explain:

- `weixin` uses a ClawBot-compatible bridge path
- `qqbot` uses an official QQ Bot or plugin bridge path
- `onebot` is a protocol bridge surface

and publish stable target examples.

### Task 6: Document the plugin manifest guidance

**Files:**
- Modify: `docs/design-docs/plugin-package-manifest-contract.md`

**Step 1: Add channel-bridge guidance**

Document how a bridge plugin should use the existing manifest seam:

- `channel_id`
- `setup.surface = "channel"`
- `setup.docs_urls`
- bridge metadata in `metadata`

**Step 2: Add concrete examples**

Show example manifest snippets for a `weixin` ClawBot bridge plugin and a
`qqbot` bridge plugin.

### Task 7: Run verification

**Files:**
- Modify: none

**Step 1: Run focused registry tests**

Run:

`cargo test -p loongclaw-app channel::registry::tests::`

**Step 2: Run broader docs-aware verification**

Run:

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`

**Step 3: Inspect the diff**

Run:

- `git status --short`
- `git diff -- docs/plans/2026-03-31-weixin-qqbot-channel-bridge-support-design.md`
- `git diff -- crates/app/src/channel/registry.rs README.md README.zh-CN.md docs/product-specs/channel-setup.md docs/design-docs/plugin-package-manifest-contract.md`

Plan complete and saved to `docs/plans/2026-03-31-weixin-qqbot-channel-bridge-support-implementation-plan.md`. Two execution options:

1. Subagent-Driven (this session) - I dispatch fresh subagent per task, review between tasks, fast iteration
2. Parallel Session (separate) - Open new session with executing-plans, batch execution with checkpoints
