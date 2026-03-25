# Plugin Package Manifest And Setup Contract

## Purpose

LoongClaw already has a real plugin intake path:

- source scanning through `PluginScanner`
- bridge/runtime translation through `PluginTranslator`
- activation planning through `PluginActivationPlan`
- policy-bounded apply/defer decisions through `PluginBootstrapExecutor`

That baseline is useful, but it is still one layer short of a durable ecosystem
contract. This document defines the next contract layer:

- manifest-first package metadata
- setup-only plugin surfaces for onboarding and doctor
- capability-slot ownership semantics for plugin-provided runtime surfaces

The intent is to learn from OpenClaw's manifest-first shape without copying its
in-process trust model.

## Why This Contract Exists

The current `PluginManifest` path in `crates/kernel/src/plugin.rs` is still
source-oriented:

- manifests are discovered from marker-delimited comment blocks
- discovery depends on parsing source files instead of package metadata
- setup and onboarding cannot consume plugin metadata without scanning source
- ownership conflicts are inferred indirectly from ids instead of explicit slot
  semantics

That shape is acceptable while plugin intake remains internal or experimental.
It will not scale cleanly to:

- third-party package distribution
- setup-time provider/channel guidance
- manifest-driven plugin catalogs
- deterministic conflict handling for shared vs exclusive plugin surfaces

OpenClaw's best lesson here is structural, not runtime-specific:

- package metadata should be a first-class contract
- setup should be separable from full runtime activation
- ownership semantics should be explicit instead of implicit

LoongClaw should absorb those lessons while preserving its stronger
kernel-governed safety boundary.

## Current Baseline

Today LoongClaw already proves several important building blocks:

- `PluginManifest` carries typed identity and metadata
- `PluginIR` normalizes multi-language plugin intake into a bridge-neutral form
- `BridgeSupportMatrix` blocks unsupported bridge and adapter profiles
- `BootstrapPolicy` keeps plugin activation policy-driven and auditable
- Roadmap stages already call for community plugin intake, signing, and trust
  tiers

The missing piece is the package contract that sits before translation and
before bootstrap.

## Non-Goals

This contract does not:

- switch LoongClaw to untrusted in-process native plugins by default
- replace kernel registry or policy ownership with plugin-owned runtime policy
- force every plugin onto the same runtime bridge
- solve marketplace distribution, signing, or supply-chain trust by itself
- replace the existing source-marker intake path in one breaking step

Those concerns are follow-on work. This contract exists so those later steps
share one metadata and ownership model.

## Contract

### 1. Package Manifest Owns Plugin Identity

Every distributable plugin package should have one package-level manifest file.

Recommended filename:

- `loongclaw.plugin.json`

The manifest is the source of truth for:

- canonical `plugin_id`
- version and display metadata
- provided runtime surfaces
- bridge/runtime metadata
- setup metadata
- capability-slot ownership declarations

Source-embedded marker blocks remain valid during migration, but they become a
compatibility input rather than the preferred contract.

### 2. Discovery Is Manifest-First And Additive

Discovery precedence should be:

1. package manifest file
2. embedded source manifest block

If both exist for the same package root:

- the package manifest is authoritative
- embedded source metadata may fill only explicitly-compatible optional fields
- conflicting values fail discovery with a typed reason instead of silently
  merging

This keeps the migration additive while preventing hidden package drift.

### 3. Setup Is A Separate Surface From Runtime Activation

Each plugin package may expose an optional `setup` section that is safe to
consume before the runtime bridge is activated.

The setup contract should support two modes:

- `metadata_only`
- `governed_entry`

`metadata_only` is the default and should carry:

- required environment variable names
- recommended environment variable names
- required config keys
- onboarding surface hints such as `web_search`, `channel`, or `memory`
- documentation links or remediation copy

`governed_entry` is optional and should:

- run through an explicit bridge contract
- respect the same policy and audit boundaries as any other plugin execution
- never imply in-process trust
- stay focused on setup/health actions rather than full runtime service

Onboarding, install, and doctor should be able to render setup guidance from
manifest metadata alone. Executing a governed setup entry should be an explicit
second step, not a prerequisite for discovery.

### 4. Ownership Uses Capability Slots, Not Hidden Conventions

Plugin packages should declare the runtime surfaces they own through explicit
slot declarations instead of only through loosely-related ids.

A slot declaration should contain:

- `slot`
- `key`
- `mode`

Recommended modes:

- `exclusive`
- `shared`
- `advisory`

Examples:

- `provider:web_search` + `tavily` + `exclusive`
- `channel:telegram` + `default` + `exclusive`
- `tool:search` + `web` + `shared`
- `memory:indexer` + `vector` + `advisory`

The important distinction is that raw capabilities and ownership are not the
same thing:

- capabilities describe what the plugin is allowed to do
- slots describe which runtime surface the plugin intends to own or extend

That separation prevents the registry and bootstrap layers from inferring
product ownership from low-level execution capabilities.

### 5. Registry Remains Kernel-Owned

The package manifest must feed the registry. It must not replace the registry.

Registry-owned behavior remains responsible for:

- canonical runtime ids
- effective selection order
- operator-facing grouped inventory
- final conflict resolution
- policy-bound activation state

Manifest data is inventory input. The kernel and registry remain the final
control plane for what becomes active.

### 6. Translation And Bootstrap Stay Deterministic

The manifest-first contract should feed the existing translation and bootstrap
pipeline in this order:

1. discover package manifest
2. normalize setup and ownership metadata
3. evaluate slot conflicts
4. translate bridge/runtime profile
5. run security scan and activation planning
6. apply, defer, or block through bootstrap policy

This avoids a future state where setup, bridge translation, and activation
policy each invent their own plugin metadata parsing rules.

### 7. Untrusted Extensions Stay On Controlled Execution Lanes

This contract should explicitly preserve LoongClaw's preferred extension lanes:

- WASM runtime lane
- process bridge lane
- MCP server lane
- ACP bridge/runtime lanes
- HTTP JSON bridge lane when policy allows it

It should explicitly reject the assumption that third-party plugins should run
in-process with the daemon by default.

The package contract is about metadata, discovery, setup, and ownership. It is
not a reason to weaken runtime isolation.

## Recommended Manifest Shape

The initial file contract should stay close to the existing `PluginManifest`
shape and grow additively.

Example:

```json
{
  "api_version": "v1alpha1",
  "plugin_id": "tavily-search",
  "version": "0.1.0",
  "provider_id": "tavily",
  "connector_name": "tavily-http",
  "summary": "Web search provider package for Tavily-backed search.",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search",
    "entrypoint": "https://api.tavily.com/search"
  },
  "setup": {
    "mode": "metadata_only",
    "surface": "web_search",
    "required_env_vars": ["TAVILY_API_KEY"],
    "default_env_var": "TAVILY_API_KEY"
  },
  "slots": [
    {
      "slot": "provider:web_search",
      "key": "tavily",
      "mode": "exclusive"
    }
  ],
  "tags": ["search", "provider", "web"]
}
```

Important design constraints:

- flat fields used by the current `PluginManifest` remain readable
- nested sections such as `setup` and `slots` are additive
- `metadata` remains available for bridge-specific details that do not yet
  deserve first-class schema fields

## Migration Plan

### Phase 1: File Contract Without Breaking Source Markers

- add package-manifest file parsing
- preserve source-marker parsing as a fallback
- define precedence and conflict errors

### Phase 2: Setup Metadata Surfaces

- expose setup metadata to onboarding, install, and doctor
- add guided setup rendering without executing plugin runtime
- introduce governed setup entries only for the cases that need active probing

### Phase 3: Slot-Aware Activation

- teach activation planning and registry projection about ownership slots
- distinguish shared vs exclusive surfaces
- emit typed conflict and precedence diagnostics

### Phase 4: Supply-Chain And SDK Alignment

- align package contract with trust-tier, signing, and provenance work
- align SDK work with the manifest contract rather than inventing a parallel
  author-facing metadata model

## Relationship To Existing RFCs

This contract should be treated as an upstream architecture layer for:

- `#425` WASM Host Function ABI
- `#426` Plugin SDK Crate

Those RFCs define execution and authoring surfaces. This document defines the
package metadata and ownership contract they should target.

It also supports the broader goals in `#292` without forcing the current
registry-first design to regress into a plugin-owned runtime model.

## Anti-Patterns

The following patterns violate this contract:

- treating source comment extraction as the long-term primary package contract
- requiring plugin runtime execution just to render setup guidance
- inferring exclusive ownership from ids without a declared slot model
- letting plugin manifests directly widen kernel policy or pack boundaries
- copying OpenClaw's in-process trust model into LoongClaw as the default
  extension path
- introducing separate metadata shapes for discovery, setup, translation, and
  SDK authoring

## Validation Standard

Any change that implements this contract should verify:

- manifest discovery precedence and conflict handling
- setup rendering without runtime execution
- slot conflict behavior for exclusive vs shared surfaces
- deterministic translation and bootstrap decisions from the normalized
  manifest
- policy/audit evidence for any governed setup execution path

For doc-only changes, the minimum repository checks should include:

- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`

## Future Direction

The long-term target is not "more plugin magic". The target is a plugin
ecosystem that remains:

- discoverable through package metadata
- guided through setup metadata
- governable through slot-aware registry ownership
- safe through controlled execution lanes

That is the smaller correct path from today's registry-first baseline to a real
community plugin platform.
