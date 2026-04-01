# Channel Bridge Plugin Contract Implementation Plan

## Goal

Turn the documented bridge-plugin manifest conventions for `weixin`, `qqbot`,
and `onebot` into a typed, validated, and operator-visible contract.

## Architecture

Use the narrowest end-to-end path that matches current ownership boundaries:

1. derive the bridge contract in `kernel` from existing manifest fields
2. project that contract through `spec` metadata and tool search
3. publish registry-owned bridge expectations in `app`
4. expose the result through `daemon` channel inventory JSON

This plan intentionally avoids a new manifest schema and avoids plugin-root
discovery in the channel CLI.

## Implementation Steps

### 1. Persist the design and analysis artifacts

Files:

- `docs/plans/2026-04-01-channel-bridge-plugin-contract-design.md`
- `docs/plans/2026-04-01-channel-bridge-plugin-contract-implementation-plan.md`
- maintainer-local private KB analysis note (not repo-tracked)

Store the public design in the repo and the deeper reasoning in the private KB.

### 2. Add kernel-side failing tests for contract derivation

Files:

- `crates/kernel/src/plugin_ir.rs`

Cover:

- valid derived bridge contract
- incomplete derived bridge contract
- activation blocked for invalid explicit contract

### 3. Implement typed contract derivation in `kernel`

Files:

- `crates/kernel/src/plugin_ir.rs`
- `crates/kernel/src/lib.rs`

Add:

- `PluginChannelBridgeContract`
- `PluginChannelBridgeReadiness`
- `BlockedInvalidManifestContract`

Derive the contract from:

- `channel_id`
- `setup.surface`
- `metadata.transport_family`
- `metadata.target_contract`
- `metadata.account_scope`

### 4. Project the contract through `spec`

Files:

- `crates/spec/src/spec_execution.rs`
- `crates/spec/src/spec_execution/tool_search.rs`
- `crates/spec/src/spec_runtime.rs`

Add bridge-contract metadata enrichment and tool-search fields for:

- channel identity
- transport family
- target contract
- account scope
- readiness
- missing field list

### 5. Publish registry-owned bridge expectations in `app`

Files:

- `crates/app/src/channel/registry.rs`
- `crates/app/src/channel/mod.rs`

Add:

- `ChannelPluginBridgeContract`
- `ChannelPluginBridgeManifestValidation`
- `validate_plugin_channel_bridge_manifest`
- `plugin_bridge_contract` on `ChannelCatalogEntry`

Make the registry contract explicit about required vs recommended metadata:

- required metadata keys: `transport_family`, `target_contract`
- recommended additive metadata keys: `bridge_kind`, `adapter_family`,
  `entrypoint`, `account_scope`

The app-layer validator should reject explicit bridge manifests that omit the
required metadata keys so the public contract matches the kernel readiness
gate.

Keep the contract registry-owned and derive it from existing plugin-backed
channel descriptors rather than per-channel custom logic.

### 6. Expose the contract through channel inventory output

Files:

- `crates/daemon/tests/integration/mod.rs`

Because the channel payload already serializes `ChannelCatalogEntry`, the code
change is primarily in the registry model and JSON coverage.

### 7. Verification

Focused verification:

- `cargo test -p loongclaw-kernel --locked plugin_ir::tests:: -- --nocapture`
- `cargo test -p loongclaw-spec --locked spec_execution::tool_search::tests::execute_tool_search_surfaces_channel_bridge_contract_fields -- --nocapture`
- `cargo test -p loongclaw-spec --locked spec_execution::tool_search::tests::execute_tool_search_surfaces_incomplete_channel_bridge_contract_fields -- --nocapture`
- `cargo test -p loongclaw-spec --locked plugin_metadata_tests::enrich_scan_report_adds_channel_bridge_contract_metadata -- --nocapture`
- `cargo test -p loongclaw-app --locked channel::registry::tests::resolve_channel_catalog_entry_exposes_plugin_bridge_contracts -- --nocapture`
- `cargo test -p loongclaw-app --locked channel::registry::tests::validate_plugin_channel_bridge_manifest_reports_contract_mismatches -- --nocapture`
- `cargo test -p loongclaw-daemon --locked build_channels_cli_json_payload_includes_plugin_bridge_contracts -- --nocapture`

Repository verification before delivery:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

## Delivery Notes

Keep the final commit limited to the bridge-contract slice and link the GitHub
issue and PR with the repository templates. Use file-backed `gh` bodies to
avoid shell-corrupting markdown.
