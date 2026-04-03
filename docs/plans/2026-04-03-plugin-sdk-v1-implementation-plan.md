# Plugin SDK V1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Tighten the plugin package contract by adding additive v1 manifest
fields, slot declarations, and explicit bridge metadata validation for package
manifests without breaking embedded-source compatibility.

**Architecture:** Keep the existing `PluginScanner -> PluginTranslator ->
PluginActivationPlan -> PluginBootstrapExecutor` flow intact. Land the SDK v1
contract as a narrow kernel-level slice in `crates/kernel/src/plugin.rs`, then
verify that translation and activation continue to behave correctly for
embedded-source compatibility paths.

**Tech Stack:** Rust, serde, existing kernel plugin pipeline, cargo test,
cargo fmt, cargo clippy

---

## Execution Tasks

### Task 1: Land the design artifacts

**Files:**
- Create: `docs/plans/2026-04-03-plugin-sdk-v1-design.md`
- Create: `docs/plans/2026-04-03-plugin-sdk-v1-implementation-plan.md`

**Step 1: Write the artifacts**

- capture the narrowed `provider/connector bridge package SDK` scope
- define additive v1 manifest fields
- define slot declarations and modes
- define package-manifest explicit bridge metadata requirements

**Step 2: Verify the artifacts exist**

Run:

```bash
test -f docs/plans/2026-04-03-plugin-sdk-v1-design.md
test -f docs/plans/2026-04-03-plugin-sdk-v1-implementation-plan.md
```

Expected: success

### Task 2: Add failing manifest-schema tests

**Files:**
- Modify: `crates/kernel/src/plugin.rs`

**Step 1: Write the failing tests**

Add tests with a `plugin_sdk_v1_` prefix that prove:

- package manifests can parse `api_version`, `version`, `display_name`, and
  `slots`
- slot declarations are normalized deterministically
- package manifests fail when `metadata.bridge_kind` is missing
- package manifests fail when `metadata.adapter_family` is missing
- package manifests fail when `metadata.entrypoint` is missing
- embedded-source manifests keep compatibility behavior without the new package
  validation gate

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p loongclaw-kernel plugin_sdk_v1_ -- --nocapture
```

Expected: FAIL because the current manifest model does not store the new fields
or enforce package-manifest bridge metadata requirements.

### Task 3: Add the v1 manifest types and normalization

**Files:**
- Modify: `crates/kernel/src/plugin.rs`
- Modify: `crates/kernel/src/lib.rs`

**Step 1: Write minimal implementation**

Add additive manifest types:

- `PluginManifestSlot`
- `PluginManifestSlotMode`

Extend `PluginManifest` with:

- `api_version: Option<String>`
- `version: Option<String>`
- `display_name: Option<String>`
- `slots: Vec<PluginManifestSlot>`

Normalize:

- optional strings
- slot string fields
- slot deduplication
- package-manifest version propagation into `metadata.version` for compatibility

**Step 2: Run the focused tests**

Run:

```bash
cargo test -p loongclaw-kernel plugin_sdk_v1_ -- --nocapture
```

Expected: still FAIL until the package-manifest validation gate exists.

### Task 4: Add explicit bridge metadata validation for package manifests

**Files:**
- Modify: `crates/kernel/src/plugin.rs`

**Step 1: Write minimal implementation**

For `parse_package_manifest_file`, reject package manifests that do not contain:

- `metadata.bridge_kind`
- `metadata.adapter_family`
- `metadata.entrypoint`

Surface failures as `IntegrationError::PluginManifestParse` with a deterministic
reason that names the missing field.

Do not apply the same strict validation to embedded-source manifests in this
slice.

**Step 2: Run the focused tests**

Run:

```bash
cargo test -p loongclaw-kernel plugin_sdk_v1_ -- --nocapture
```

Expected: PASS

### Task 5: Re-run existing plugin pipeline regression tests

**Files:**
- No new files

**Step 1: Run kernel regressions**

Run:

```bash
cargo test -p loongclaw-kernel scanner_finds_package_manifest_file -- --nocapture
cargo test -p loongclaw-kernel scanner_prefers_package_manifest_over_embedded_source_manifest -- --nocapture
```

Expected: PASS

**Step 2: Run daemon regression**

Run:

```bash
cargo test -p loongclaw-daemon execute_spec_tool_search_uses_explicit_plugin_setup_readiness_context -- --nocapture
```

Expected: PASS

### Task 6: Run finish-line verification

**Files:**
- Modify: `crates/kernel/src/plugin.rs`
- Modify: `crates/kernel/src/lib.rs`
- Create: `docs/plans/2026-04-03-plugin-sdk-v1-design.md`
- Create: `docs/plans/2026-04-03-plugin-sdk-v1-implementation-plan.md`

**Step 1: Run formatting**

Run:

```bash
cargo fmt --all
```

Expected: success

**Step 2: Run targeted verification**

Run:

```bash
cargo test -p loongclaw-kernel plugin_sdk_v1_ -- --nocapture
cargo test -p loongclaw-kernel scanner_finds_package_manifest_file -- --nocapture
cargo test -p loongclaw-kernel scanner_prefers_package_manifest_over_embedded_source_manifest -- --nocapture
cargo test -p loongclaw-daemon execute_spec_tool_search_uses_explicit_plugin_setup_readiness_context -- --nocapture
```

Expected: PASS
