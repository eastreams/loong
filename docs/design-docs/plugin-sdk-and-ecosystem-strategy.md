# Plugin SDK And Ecosystem Strategy

## Purpose

LoongClaw already has two strong plugin foundations:

- a strict native package contract through `PluginManifest`
- a foreign-dialect compatibility seam through `PluginDescriptor`, `PluginIR`,
  `PluginActivationPlan`, and bridge-support policy

Those foundations are necessary, but not sufficient, for a mature plugin
ecosystem.

A durable ecosystem also needs a clear answer to four product questions:

1. What contract should native LoongClaw plugin authors target?
2. How should OpenClaw-compatible packages enter the system without polluting
   the kernel?
3. What metadata shape should marketplace, installation, onboarding, and UI
   surfaces consume?
4. How do SDKs, migration tooling, and operator governance stay aligned instead
   of drifting into parallel policy engines?

This document defines the missing strategy layer.

It complements, rather than replaces:

- [Plugin Package Manifest Contract](plugin-package-manifest-contract.md)
- [OpenClaw Plugin Compatibility Contract](openclaw-plugin-compatibility-contract.md)

Those documents define the package and compatibility boundaries.
This document defines the ecosystem, SDK, migration, and marketplace shape that
should grow on top of those boundaries.

## Design Goal

LoongClaw should become:

- native-first for authoring and long-term host evolution
- compatibility-capable for OpenClaw and future foreign ecosystems
- explicit about marketplace and installation policy
- governable through one preflight, activation, and attestation truth

It should not become:

- an in-process OpenClaw runtime clone
- a marketplace-only shell with no host-level governance
- a pile of ecosystem-specific SDKs that bypass kernel contracts

## Executive Summary

The mature ecosystem shape has four layers:

1. **Native package contract**
   - one stable host-facing plugin contract
   - one typed manifest-first package shape
2. **Marketplace and interface contract**
   - one user-facing catalog shape for install, auth, category, and display
   - decoupled from kernel activation semantics
3. **Runtime bridge contract**
   - one bridge-oriented execution boundary for `process_stdio`, `http_json`,
     `mcp_server`, future WASM, and related lanes
4. **SDK and migration contract**
   - one native SDK family for LoongClaw packages
   - one compatibility SDK family for OpenClaw ingestion and migration

The core architectural move is simple:

- native LoongClaw remains the primary authoring target
- OpenClaw enters as a foreign dialect
- compatibility is translated into canonical LoongClaw truth before activation
- marketplace and SDK surfaces consume that same truth instead of inventing
  new metadata models

## What OpenClaw Teaches

OpenClaw's strongest lesson is not "copy the runtime".
Its strongest lesson is that plugin ecosystems become useful only when package
metadata, setup metadata, and author-facing helpers are real product surfaces.

OpenClaw is worth learning from in three areas:

### 1. Manifest-first plugin identity

`openclaw.plugin.json` lets OpenClaw validate plugin configuration and discover
plugin-owned surfaces without executing plugin code.

LoongClaw should keep absorbing that lesson:

- package metadata must stay manifest-first
- setup and doctor should not require runtime execution
- plugin identity should not be inferred from runtime side effects

### 2. Rich authoring helpers

OpenClaw's `openclaw/plugin-sdk/*` surface is large because it optimizes for
plugin-author speed.

That is useful, but it also means:

- SDK surface area grows quickly
- host internals become part of the authoring contract
- host refactors become expensive

LoongClaw should learn the positive lesson without taking on the same coupling:

- authoring helpers should be rich
- kernel contracts should remain narrow
- helper libraries should be layered above the contract, not fused into it

### 3. Optional tool exposure policy

OpenClaw's optional plugin tools and allowlist policy are worth reusing at the
architectural level.

The important principle is:

- a package may declare a tool
- that does not mean the model may automatically call it

LoongClaw should keep this principle, but enforce it through kernel-visible
exposure policy instead of runtime-only plugin conventions.

## What Marketplace-Oriented Plugin Systems Teach

Marketplace-oriented local plugin systems are more useful as packaging and
catalog references than as runtime plugin references.

The durable lessons are structural rather than file-layout specific:

### 1. Marketplace policy should be separate from runtime contract

A catalog entry should answer:

- where a package comes from
- whether it is installable
- when authentication or setup is required
- how it is categorized and presented

That is a different concern from:

- what the kernel may activate
- which bridges or shims are allowed
- what compatibility mode is required

### 2. Plugin bundles should compose existing product surfaces

A package may carry more than one kind of contribution.
It can combine runtime bridges with operator and user-facing surfaces such as:

- setup flows
- tool and service integrations
- app-facing connectors
- discovery and onboarding metadata

LoongClaw should therefore treat marketplace packaging as a composition surface,
not only as a bridge-execution surface.

### 3. Interface metadata deserves a stable home

Display names, categories, install guidance, and related discovery UX belong in
a typed interface contract.
They should not be squeezed into kernel-only runtime metadata.

## Architectural Principle

LoongClaw should be **native-first, bridge-compatible, and marketplace-aware**.

That principle implies three rules:

1. **Native-first authoring**
   - new LoongClaw packages should target the native package contract and native
     SDKs first
2. **Bridge-compatible foreign intake**
   - OpenClaw and future ecosystems should enter through descriptor
     normalization plus bridge-support policy
3. **Marketplace-aware packaging**
   - operator and user-facing install/catalog surfaces should live in a typed
     marketplace contract instead of leaking through kernel metadata

## Layered Ecosystem Model

### Layer 1: Native Package Contract

The native package contract is the first-class host contract.

Its responsibilities are:

- stable identity
- versioning
- setup metadata
- slot ownership
- host compatibility declarations
- capability and bridge metadata
- attestation inputs

This layer should stay small and typed.
It is the target for native SDK generation and native package validation.

### Layer 2: Foreign Dialect Normalization

Foreign packages should never skip normalization.

This layer is responsible for:

- dialect detection
- dialect provenance
- canonical descriptor projection
- compatibility-mode selection
- foreign diagnostics
- migration hints

The kernel should continue to reason about one normalized shape after this
step.

### Layer 3: Runtime Bridge Contract

Bridge execution remains separate from dialect identity.

This layer is responsible for:

- `process_stdio`
- `http_json`
- `mcp_server`
- future WASM or ACP-related bridge lanes
- runtime profile validation
- execution attestation re-checks

The system should keep bridge semantics explicit and avoid ecosystem-specific
execution planes.

### Layer 4: Marketplace And Interface Contract

Marketplace metadata should sit above the kernel, not inside it.

This layer is responsible for:

- catalog source and provenance
- install policy
- auth policy
- category and discovery ranking inputs
- interface display metadata
- bundle composition metadata for skills, MCP servers, and apps

This layer may consume kernel truth, but it must not redefine it.
Marketplace metadata must stay descriptive and policy-scoped. It must never
become a second activation authority or a backdoor path around preflight,
bridge-policy, or attestation checks.

### Layer 5: Native Host Extension ABI

Not every plugin should get deep host authority.

A narrower native host-extension ABI should exist only for cases where a
package truly needs:

- a native connector adapter
- a native memory adapter
- a native runtime adapter
- tightly-scoped typed hook points

This lane should remain smaller and more controlled than OpenClaw's broad
in-process registration surface. It should be trusted, narrow, and non-default,
with explicit operator intent before any package gains deeper host authority.

## SDK Family Strategy

LoongClaw should not ship one giant plugin SDK.
It should ship a family of SDK layers.

The layer names below are illustrative, not a locked future crate or package
layout. The architectural commitment is to keep contract, runtime-helper, and
compatibility-helper concerns separate. The exact packaging can still evolve as
long as it preserves that separation and continues to target the same manifest
and activation contracts.

### 1. `loongclaw-sdk-contract`

Purpose:

- expose the stable package, bridge, setup, diagnostics, and governance types
- let external tooling reuse one schema surface

Primary consumers:

- CI and release tooling
- marketplace validators
- migration tools
- policy and preflight automation
- native or foreign package generators

This layer should remain narrow and versioned. SDK generators, CI, and
marketplace tooling should inherit the same strict manifest contract instead of
introducing a parallel metadata model.

### 2. `loongclaw-sdk-runtime`

Purpose:

- help plugin authors implement bridge-style packages cleanly
- provide structured helpers above the stable contract

Primary helpers should include:

- stdio bridge helpers
- HTTP JSON bridge helpers
- MCP bridge helpers
- state and temp-path helpers
- structured logging and health helpers
- setup and capability declaration helpers

This layer may be richer than the contract layer.
The key rule is that it must build on the contract instead of redefining it.

### 3. `loongclaw-sdk-openclaw-compat`

Purpose:

- help OpenClaw packages migrate or integrate without contaminating the native
  SDK shape

Primary helpers should include:

- OpenClaw manifest parsing
- metadata projection helpers
- compatibility shim helpers
- migration scaffolding
- compatibility diagnostics and linting

This layer should be explicit about being transitional.
It is a compatibility lane, not the preferred authoring target.

## Marketplace Contract Strategy

LoongClaw should define one explicit marketplace contract that is separate from
`loongclaw.plugin.json`.

A marketplace entry should answer:

- `marketplace_id`
- package source and provenance
- install policy
- auth policy
- category
- interface metadata overrides or additions
- trust/review tier
- compatibility posture
- host-version or product gating where needed

The marketplace contract should be able to describe both:

- native LoongClaw packages
- imported foreign packages that still enter through compatibility lanes

That makes it possible to list an OpenClaw-compatible package without claiming
that it is native.

## Capability And Tool Exposure Strategy

Tool declaration and model exposure should be distinct.

The desired flow is:

1. package declares tool surfaces
2. translation and activation normalize those surfaces
3. preflight evaluates risk and support posture
4. host policy decides whether the tool is exposed to the model

This keeps the system aligned with the kernel-first design:

- plugin metadata may request surfaces
- only the host policy may expose them

Optional or higher-risk tools should be opt-in by default.

## Compatibility Levels

OpenClaw compatibility should be described in explicit levels rather than one
vague promise of "full compatibility".

### Level 0: Discovery Compatibility

LoongClaw can:

- detect the package
- classify dialect and provenance
- inventory it
- preflight it

### Level 1: Packaging Compatibility

LoongClaw can additionally:

- project package metadata
- preserve setup guidance
- preserve install and UI hints
- expose the package through marketplace and operator surfaces

### Level 2: Bridge Runtime Compatibility

LoongClaw can additionally execute the package through supported bridge lanes,
starting with:

- `process_stdio`
- `http_json`
- `mcp_server`

### Level 3: Semantic Shim Compatibility

LoongClaw can additionally emulate or adapt selected host-level OpenClaw plugin
semantics, such as a constrained registration subset.

This level should be incremental and explicit.
It is the most expensive compatibility layer and should not be treated as the
baseline requirement for ecosystem usefulness.

## Migration Strategy

The migration story should be a first-class product surface.

A mature ecosystem should support three flows:

### 1. Native-first authoring

New packages start from the native LoongClaw contract and native SDK.

### 2. Compatibility intake

Existing OpenClaw packages can be:

- discovered
- inventoried
- preflighted
- gated by runtime bridge profiles
- installed into a catalog without pretending they are native

### 3. Guided migration

A future migration CLI should be able to:

- read an OpenClaw package
- emit a native `loongclaw.plugin.json` scaffold
- preserve compatibility metadata and setup hints
- emit a migration report with explicit TODO items

That gives LoongClaw a healthy ecosystem funnel:

- compatible by discovery
- useful by bridge execution
- durable by migration to native

## Operator Surface Strategy

Operator surfaces should stay thin wrappers over canonical truth.

The operator should not need to re-derive ecosystem state from raw manifests.
The existing direction is correct:

- `plugin_inventory` surfaces package and activation truth
- `plugin_preflight` surfaces governance truth
- `plugins bridge-profiles` surfaces compatibility presets
- attestation keeps runtime execution aligned with approved activation

Future ecosystem tooling should continue to build on those same surfaces.

## Recommended Implementation Order

### Phase 1: Strategy Closure

- keep the native package contract authoritative
- keep OpenClaw normalization explicit and fail-closed
- define the marketplace and interface contract separately from kernel metadata
- define the SDK family boundaries explicitly

### Phase 2: Runtime Compatibility MVP

Start with the bridge lanes that preserve the cleanest safety boundary:

- modern OpenClaw manifests
- `process_stdio`
- `http_json`
- `mcp_server`

Do not make broad in-process semantic emulation the first milestone.

### Phase 3: Marketplace And Migration Tooling

- define marketplace schema and validation
- define package listing and install policy semantics
- add migration scaffolding for OpenClaw packages
- align docs, preflight outputs, and future SDK generators on one metadata
  model

### Phase 4: Narrow Native Host Extension ABI

After the native package and bridge lanes are stable:

- define the minimum native host-extension ABI
- expose only typed, policy-bounded extension points
- avoid copying OpenClaw's broad runtime registration model

## Anti-Patterns

The following patterns should be avoided:

- treating OpenClaw host semantics as the native LoongClaw authoring target
- putting marketplace or display metadata into kernel-only runtime fields
- building a parallel SDK metadata model that disagrees with package manifests
- auto-enabling foreign compatibility just because discovery succeeded
- exposing declared plugin tools to the model without separate host approval
- equating bridge execution support with full semantic host compatibility
- letting compatibility shims widen kernel policy implicitly

## Validation Standard

Any change that claims to advance this strategy should verify:

- native package contracts remain the first-class authoring target
- foreign dialects still normalize into canonical activation truth
- marketplace or SDK tooling consumes that canonical truth instead of inventing
  a parallel model
- bridge-support policy remains the single runtime compatibility gate
- optional or high-risk tool surfaces require explicit exposure policy
- migration tooling preserves provenance and compatibility context

For doc-only changes, the minimum repository checks should include:

- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`

## Decision

LoongClaw should mature its plugin ecosystem through:

- one native package contract
- one explicit foreign-dialect normalization seam
- one bridge-first runtime execution boundary
- one separate marketplace and interface contract
- one layered SDK family with native and compatibility lanes
- one migration funnel from ecosystem compatibility into native packages

That is the smallest ecosystem architecture that stays:

- native-first
- OpenClaw-compatible
- marketplace-ready
- kernel-safe
