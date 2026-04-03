# Plugin SDK V1 Design

**Problem**

LoongClaw already has a real plugin intake pipeline, but it still lacks a
stable author-facing SDK contract. Today, plugin discovery, translation,
activation planning, and bootstrap all exist, yet the authoring model remains
partially source-oriented and partially implicit.

That gap shows up in a few concrete ways:

- package manifests are real, but not yet the full external contract
- package manifests still rely too much on ad hoc `metadata`
- bridge selection can still fall back to language-based inference
- setup metadata exists, but it is not yet framed as part of a coherent SDK
- ownership intent for plugin-provided runtime surfaces is still mostly
  implicit

If LoongClaw wants a third-party plugin ecosystem without weakening runtime
governance, the first stable SDK boundary should be the package contract, not a
large in-process runtime API.

## Current Architecture Evidence

The current repository already contains the main pieces needed for a
manifest-first SDK:

- package-manifest discovery in
  `crates/kernel/src/plugin.rs`
- bridge/runtime normalization in
  `crates/kernel/src/plugin_ir.rs`
- activation planning with setup-readiness evaluation in
  `crates/kernel/src/plugin_ir.rs`
- policy-bounded apply or defer decisions in
  `crates/kernel/src/bootstrap.rs`
- end-to-end spec runtime orchestration in
  `crates/spec/src/spec_execution.rs`

The package-manifest contract doc in
`docs/design-docs/plugin-package-manifest-contract.md`
already points in the correct direction:

- manifest-first authoring
- setup metadata as a pre-runtime surface
- slot-aware ownership declarations
- controlled execution lanes instead of default in-process trust

The missing piece is not another high-level ambition document. The missing
piece is a narrower v1 SDK contract that matches what the repository can
actually implement now.

## Design Goals

1. Define a stable v1 author-facing package contract.
2. Keep the SDK aligned with the existing kernel plugin pipeline.
3. Require explicit bridge metadata for package manifests.
4. Preserve source-marker intake as a compatibility path, not the preferred
   authoring path.
5. Introduce slot declarations as stable metadata before wiring full slot
   conflict resolution.
6. Preserve LoongClaw's controlled execution lanes and avoid making native FFI
   the default third-party path.

## Non-goals

- Do not ship a unified external SDK for provider, channel, tool, and memory
  authoring in v1.
- Do not freeze app-native host traits such as channel adapters or memory
  systems as stable external ABI.
- Do not replace the existing scan, translate, activation, and bootstrap
  pipeline.
- Do not require `governed_entry` setup execution to be complete before the
  package contract can ship.
- Do not weaken bootstrap policy to make third-party plugin activation feel
  "automatic."

## Core Idea

Plugin SDK v1 should be a `provider/connector bridge package SDK`.

That means the stable contract is centered on:

- `loongclaw.plugin.json`
- additive manifest schema fields for package identity and display metadata
- explicit bridge runtime profile metadata
- setup metadata that can be consumed before runtime activation
- slot declarations that describe ownership intent
- validation tooling that tells authors whether their package is well-formed
  and activation-ready

The SDK does not need to begin as a unified runtime API. It only needs to make
plugin packaging, bridge targeting, and readiness validation deterministic.

## Proposed SDK Surface

### 1. Package manifest is the authoring root

Every distributable plugin package should contain one package-level manifest:

- filename: `loongclaw.plugin.json`

The package manifest becomes the preferred authoring surface for:

- plugin identity
- package version and display metadata
- bridge profile metadata
- setup metadata
- slot declarations

Embedded source manifests remain valid for compatibility, migration, and local
examples. They are not the v1 SDK's preferred external contract.

### 2. Additive manifest schema growth

The v1 package manifest should stay close to the current `PluginManifest` and
grow additively.

Keep these existing fields readable:

- `plugin_id`
- `provider_id`
- `connector_name`
- `channel_id`
- `endpoint`
- `capabilities`
- `metadata`
- `summary`
- `tags`
- `setup`

Add these v1 package fields:

- `api_version`
- `version`
- `display_name`
- `slots`

The first three fields support stable packaging and operator-facing inventory.
`slots` captures ownership intent without overloading low-level capabilities.

### 3. Explicit bridge metadata for package manifests

Package manifests must not rely on language inference for bridge selection.

For package manifests, v1 requires:

- `metadata.bridge_kind`
- `metadata.adapter_family`
- `metadata.entrypoint`

This is a hard contract change for package-manifest authoring. It does not
remove the current language-based inference in `PluginTranslator`, because that
fallback still matters for embedded-source compatibility. The important change
is that package authors should not be taught to rely on those defaults.

This is especially important because the current fallback can infer
`native_ffi` from source language for Rust or Go packages, which is not the
right default path for a third-party ecosystem.

### 4. Bridge profile lanes

The v1 SDK should treat these as the preferred extension lanes:

- `process_stdio`
- `wasm_component`
- `mcp_server`
- `http_json`
- `acp_bridge`
- `acp_runtime`

`native_ffi` remains a supported bridge kind in the runtime model, but it
should stay explicitly operator-controlled and opt-in. It should not be the
default packaging guidance or the default scaffolding choice for third-party
authors.

### 5. Setup remains a pre-runtime surface

The current `PluginSetup` model is already useful and should become part of the
v1 SDK contract.

For v1:

- `metadata_only` becomes the required stable setup mode
- `governed_entry` stays present in schema, but is not the primary delivery
  target of this slice

Setup metadata should continue to support:

- required environment variables
- recommended environment variables
- required config keys
- default env hints
- docs URLs
- remediation copy
- surface hints

This allows install, onboarding, and doctor-style surfaces to provide repair
guidance without executing plugin runtime code.

### 6. Slots stabilize ownership intent

The v1 manifest adds slot declarations:

- `slot`
- `key`
- `mode`

Supported modes:

- `exclusive`
- `shared`
- `advisory`

Example intent:

- `provider:web_search` + `tavily` + `exclusive`
- `tool:search` + `web` + `shared`
- `memory:indexer` + `vector` + `advisory`

In v1, slots do not need full runtime conflict enforcement on day one. The
important part is to stabilize the author-facing declaration shape first, then
teach activation planning and registry projection how to use it incrementally.

### 7. Host/runtime integration remains kernel-owned

The existing host pipeline remains authoritative:

1. scan package manifests and embedded source manifests
2. normalize manifest metadata
3. translate into bridge-neutral IR
4. evaluate setup readiness and bridge support
5. bootstrap through policy-controlled apply or defer logic

The package manifest feeds this pipeline. It does not replace host-side
registry ownership or bootstrap policy.

This means app-native traits such as channel adapters and memory systems remain
internal host surfaces for now. They can become future SDK layers later, but
they should not be frozen as part of this v1 package contract.

## Validation Model

The SDK must expose a concrete validation story, not just a schema file.

The minimum v1 validation standard should cover:

### Schema validation

- `api_version` is present and supported
- `version` and `display_name` normalize deterministically
- `slots` contain valid `slot`, `key`, and `mode` values
- `setup.mode` is valid

### Package bridge validation

For `PluginSourceKind::PackageManifest`, discovery should fail if:

- `metadata.bridge_kind` is absent
- `metadata.adapter_family` is absent
- `metadata.entrypoint` is absent

Embedded-source manifests keep the current compatibility behavior and may still
flow through translation defaults.

### Activation preflight

The existing activation-plan model should remain the readiness lens:

- `ready`
- `setup_incomplete`
- `blocked_unsupported_bridge`
- `blocked_unsupported_adapter_family`

The author-facing contract should point package authors toward fixing manifest
metadata first, then setup requirements, instead of hiding those states behind
runtime surprises.

## Testing Standard

The first implementation slice should prove:

1. package manifests can carry the new additive fields without breaking current
   behavior
2. package manifests fail discovery when explicit bridge metadata is missing
3. embedded-source manifests continue to use compatibility defaults
4. slot declarations are parsed and normalized deterministically
5. setup metadata remains readable and continues to affect activation planning

## Rollout Strategy

### Phase 1: Package manifest contract tightening

- add additive v1 manifest fields
- add slot schema types
- require explicit bridge metadata for package manifests
- preserve embedded-source compatibility

### Phase 2: Validation and reporting

- improve scan-time error reasons for malformed package manifests
- expose normalized slot metadata in scan outputs and tests
- align examples and docs with explicit bridge metadata

### Phase 3: Slot-aware activation follow-up

- teach activation and absorb stages how to reason about slot conflicts
- add registry projection rules for exclusive versus shared ownership

## Why This Is The Right v1

This design keeps the first SDK boundary small, enforceable, and compatible
with the repository's actual architecture.

It does not promise a unified everything-SDK before the host surfaces are
ready. It does not weaken the runtime isolation model for the sake of ecosystem
marketing. It takes the part of the system that already exists, makes the
authoring contract explicit, and sets up later slot-aware and trust-aware
follow-up work on a stable metadata base.
