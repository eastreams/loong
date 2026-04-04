# Channel Bridge Plugin Contract Design

## Scope

This slice deepens LoongClaw's support for plugin-backed `weixin`, `qqbot`,
and `onebot` channel surfaces by making their external plugin contract typed,
validated, and discoverable.

The immediate scope is:

- derive a typed channel-bridge contract from the existing plugin manifest seam
- validate declared bridge contracts before activation treats them as ready
- project the contract into spec metadata and tool-search output
- publish a registry-owned bridge contract for plugin-backed channel surfaces
- surface that registry contract through `loongclaw channels --json`

This slice does not:

- add a new top-level plugin manifest schema
- scan plugin roots from `loongclaw channels` or `loongclaw doctor`
- auto-install or auto-activate bridge plugins into channel runtime ownership
- add native `weixin`, `qqbot`, or `onebot` runtimes

## Problem Statement

LoongClaw already models `weixin`, `qqbot`, and `onebot` as plugin-backed
channel surfaces, and `loongclaw doctor` already treats their external runtime
ownership truthfully.

The remaining gap was the contract between those channel surfaces and external
plugins:

- plugin manifests already had `channel_id`
- plugin setup already had `setup.surface`
- plugin metadata already carried bridge hints such as `transport_family`
  and `target_contract`

But none of those fields were turned into a typed contract.

That caused two practical problems:

- incomplete bridge manifests could still look activation-ready
- discovery surfaces could not tell operators or automation which bridge
  contract a plugin actually claimed to implement

The root cause was not missing `weixin`-specific code. The root cause was that
the bridge contract existed only as a documentation convention.

## Constraints

The fix had to respect the existing architecture boundary:

- `kernel` owns plugin manifest intake and translation
- `spec` owns metadata enrichment and tool search
- `app` owns channel registry semantics
- `daemon` owns channel CLI rendering

That means the correct solution is layered:

1. derive the contract in `kernel`
2. project bridge facts through `spec`
3. publish registry-owned expectations in `app`
4. expose the resulting contract surfaces in `daemon`

This is an execution flow, not a new dependency chain. Each layer contributes
its own owned representation through existing boundaries.

It must not make `kernel` depend on `app`, and it must not duplicate the
contract into a second manifest schema.

## Reference Direction

The useful lessons from related bridge and plugin ecosystems were structural:

- manifest-first compatibility data is more stable than README-only guidance
- runtime ownership should stay explicit when an external bridge owns the
  upstream session lifecycle
- operator-facing inventory output should show contract expectations directly
  instead of forcing users to reverse-engineer them from internal code

Those lessons reinforce a narrow design: keep the contract declarative, keep
the runtime boundary explicit, and avoid channel-specific hacks.

## Approaches Considered

### 1. Add a new `channel_contracts` manifest section

Rejected because it duplicates existing manifest identity:

- `channel_id` already exists
- `setup.surface` already exists
- `metadata.transport_family` and `metadata.target_contract` already exist

Adding a second schema would create new drift without solving a new problem.

### 2. Keep everything app-side and registry-only

Rejected because it fixes only the presentation layer.

If the contract is not typed in `kernel`, activation and translation still
cannot distinguish a valid bridge declaration from an incomplete one.

### 3. Derive a typed contract from existing manifest conventions

Chosen because it is the smallest correct solution:

- no new schema
- no architecture cycle
- activation can enforce contract completeness
- discovery and channel inventory can reuse the same typed facts

## Chosen Design

### 1. Derive a typed bridge contract in `kernel`

`PluginTranslator` derives an optional `PluginChannelBridgeContract` from the
existing manifest seam:

- `channel_id`
- `setup.surface`
- `metadata.transport_family`
- `metadata.target_contract`
- `metadata.account_scope`

The contract also carries `readiness` with:

- `ready`
- `missing_fields`

The contract is only derived when the manifest clearly declares a channel
bridge surface. The declaration trigger is intentionally generic:

- `setup.surface == "channel"`
- or channel-bridge metadata is present
- or `channel_id` is paired with `adapter_family = "channel-bridge"`

This keeps the logic data-driven rather than keyed to `weixin`, `qqbot`, or
`onebot`.

### 2. Block activation for invalid declared bridge contracts

If a plugin explicitly declares a channel bridge contract but omits required
fields, activation should not treat it as ready.

This slice adds a dedicated blocked status:

- `BlockedInvalidManifestContract`

Required fields for readiness are:

- `channel_id`
- `setup.surface = "channel"`
- `metadata.transport_family`
- `metadata.target_contract`

`metadata.account_scope` remains additive guidance, not a readiness gate.

### 3. Project the contract through `spec`

Spec execution enriches manifest metadata with stable bridge-contract fields:

- `plugin_channel_id`
- `plugin_channel_bridge_transport_family`
- `plugin_channel_bridge_target_contract`
- `plugin_channel_bridge_account_scope`
- `plugin_channel_bridge_ready`
- `plugin_channel_bridge_missing_fields_json`

`ToolSearchEntry` and `ToolSearchResult` expose the same structure, and search
scoring includes those fields so queries such as `weixin`, `channel bridge`,
or a transport family can match the right plugin entry.

### 4. Publish a registry-owned contract in `app`

The channel registry publishes a serializable `plugin_bridge_contract` on
plugin-backed `ChannelCatalogEntry` values.

That contract is registry-owned, not plugin-owned. It tells plugin authors and
operators what a compatible manifest must look like for the surface:

- `manifest_channel_id`
- `required_setup_surface`
- `runtime_owner`
- `supported_operations`
- `recommended_metadata_keys`

The registry also exposes an app-layer validator that checks a
`PluginManifest` against the registry-owned bridge contract without making
`kernel` depend on `app`.

### 5. Surface the registry contract through channel inventory output

Because `ChannelCatalogEntry` is already part of the channel inventory payload,
`loongclaw channels --json` automatically gains a stable, operator-visible
bridge contract for `weixin`, `qqbot`, and `onebot`.

This is the right first visibility point because it shows:

- which `channel_id` a plugin should bind to
- that runtime ownership remains `external_plugin`
- which operations the surface expects
- which metadata keys are recommended for richer bridge discovery

## Why This Design

This design is the smallest correct step because it closes the actual contract
gap end to end:

- manifest conventions become typed data
- incomplete declarations stop looking valid
- discovery surfaces gain structured bridge facts
- channel inventory publishes a canonical registry-owned contract

It does that without introducing a second schema, without channel-specific
logic in `daemon`, and without violating existing crate boundaries.
