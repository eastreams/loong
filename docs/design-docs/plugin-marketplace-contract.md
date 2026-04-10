# Plugin Marketplace And Interface Contract

## Purpose

LoongClaw already has the lower layers of a governed plugin system:

- package identity and setup through the plugin package manifest contract
- foreign-dialect normalization through the OpenClaw compatibility contract
- activation and governance truth through preflight, bridge policy, and
  attestation

What it still lacks is a first-class contract for the layer above those
runtime-bound truths:

- catalog listing
- install policy
- authentication timing
- interface metadata
- distribution provenance
- imported registry and marketplace views

This document defines that missing marketplace layer.

It complements, rather than replaces:

- [Plugin Package Manifest Contract](plugin-package-manifest-contract.md)
- [OpenClaw Plugin Compatibility Contract](openclaw-plugin-compatibility-contract.md)
- [Plugin SDK And Ecosystem Strategy](plugin-sdk-and-ecosystem-strategy.md)

Those documents define package truth, compatibility truth, and SDK layering.
This document defines how packages are listed, presented, installed, and
curated without turning marketplace metadata into a second activation authority.

## Why This Contract Exists

The current architecture already talks about marketplace workflows in multiple
places:

- `plugin_preflight` has a `marketplace_submission` profile
- package and compatibility docs refer to future registry, importer, and
  marketplace tooling
- the SDK strategy now explicitly calls for a separate marketplace and
  interface contract

Without a dedicated contract, those higher-level concerns are at risk of
splitting into parallel metadata models:

- one shape for package authors
- one shape for CI and preflight tools
- one shape for curated catalogs
- one shape for future install UX

That drift would create the exact kind of slop debt the plugin package contract
was written to avoid.

The marketplace layer therefore needs its own typed contract, but it must stay
strictly above package and activation truth.

## Core Principle

Marketplace metadata describes packages.
It does not authorize packages.

The marketplace layer may answer questions such as:

- how should this package be displayed?
- where can it be obtained from?
- should it be installable by default or by explicit operator action?
- when is additional authentication expected?
- what curated category or trust tier should the listing advertise?

The marketplace layer must not answer questions such as:

- may this host activate the package right now?
- which compatibility mode is actually approved at runtime?
- which bridge profile is currently in force?
- whether a listing can bypass manifest, preflight, or attestation checks?

Those remain package, compatibility, and host-governance questions.

## Design Goal

LoongClaw should support a marketplace layer that is:

- descriptive instead of authority-bearing
- native-first but foreign-package-aware
- portable across local files, curated catalogs, and future remote registries
- compositional for skills, MCP servers, apps, and interface assets
- aligned with `plugin_preflight` and future marketplace submission workflows

It should not:

- duplicate package-manifest truth
- silently widen activation privileges
- hide compatibility or setup blockers behind optimistic catalog entries
- force package authors and catalog operators into unrelated metadata models

## Contract Scope

This contract covers four things:

1. the marketplace catalog root shape
2. the plugin listing entry shape
3. the interface metadata shape for display and discovery
4. the install and authentication policy vocabulary

It does not define:

- package runtime execution
- host bridge implementation
- package manifest parsing
- signature verification mechanics in detail
- registry transport or network protocol

Those concerns are separate layers.

## Recommended Artifact Shape

The marketplace contract should be serializable as a standalone file.

Recommended filename:

- `loongclaw.marketplace.json`

Recommended behavior:

- hosts may store or fetch catalogs through other mechanisms
- `loongclaw.marketplace.json` is the canonical serialized contract shape
- alternative transport layers should preserve the same typed fields instead of
  inventing a different wire schema

That keeps local curated catalogs, exported snapshots, and future remote
registries aligned on one contract.

## Contract Layers

### 1. Marketplace Catalog Root

A catalog root should describe the marketplace itself.

Minimum responsibilities:

- identify the catalog
- describe catalog provenance
- define optional display metadata for the marketplace
- carry the ordered listing set

Recommended root fields:

- `api_version`
- `marketplace_id`
- `display_name`
- `description`
- `provenance`
- `plugins`

### 2. Plugin Listing Entry

A listing entry should describe one package as it appears in a marketplace.

Minimum responsibilities:

- identify which package is being listed
- identify where the package can be obtained
- define install policy
- define authentication timing expectations
- provide display-layer metadata
- carry descriptive trust and compatibility hints

Recommended listing fields:

- `plugin_id`
- `source`
- `install_policy`
- `auth_policy`
- `interface`
- `trust`
- `compatibility`
- `bundle`

### 3. Interface Metadata Layer

Interface metadata is for display, discovery, and user guidance.

It should cover:

- display name
- summaries and long description
- category and capability tags
- website, privacy, and terms links
- logo, icon, and screenshots
- starter prompts or example tasks

This metadata may be curated or overridden by the marketplace, but only at the
interface layer. Package manifests remain the source of baseline package-authored
display metadata. Marketplace interface data is a listing-scoped overlay and
must not feed back into package normalization, preflight, activation, or
attestation truth.

### 4. Policy Overlay Layer

Marketplace policy is intentionally narrower than runtime policy.

It may define:

- installability
- authentication timing
- curation or review tier
- product or host gating for listing visibility

It must not define:

- activation approval
- capability grants
- bridge profile overrides
- package-trust bypasses

## Root Contract Shape

A marketplace root should be typed and strict enough for tooling.

Illustrative shape:

```json
{
  "api_version": "v1alpha1",
  "marketplace_id": "loongclaw-curated",
  "display_name": "LoongClaw Curated",
  "description": "Reviewed plugin listings for governed local hosts.",
  "provenance": {
    "publisher": "loongclaw-ai",
    "source": "curated-catalog",
    "url": "https://example.invalid/marketplace/loongclaw-curated"
  },
  "plugins": []
}
```

### Root field intent

- `api_version`
  - schema contract for the catalog itself
- `marketplace_id`
  - stable machine-facing identifier for the catalog
- `display_name`
  - user-facing marketplace title
- `description`
  - optional summary of the catalog's purpose and curation posture
- `provenance`
  - who publishes or curates the catalog and where it came from
- `plugins`
  - ordered listing entries

## Plugin Listing Contract Shape

Illustrative shape:

```json
{
  "plugin_id": "gmail-sync",
  "source": {
    "kind": "git_ref",
    "url": "https://github.com/example/gmail-sync",
    "ref": "v0.4.2"
  },
  "install_policy": "operator_install",
  "auth_policy": "on_first_use",
  "interface": {
    "display_name": "Gmail Sync",
    "short_description": "Search and act on Gmail data through governed tools.",
    "category": "Productivity",
    "capabilities": ["Read", "Search"],
    "starter_prompts": [
      "Summarize my unread mail.",
      "List threads from the last 24 hours."
    ]
  },
  "trust": {
    "review_tier": "reviewed",
    "publisher": "example"
  },
  "compatibility": {
    "declared_dialect": "loongclaw_package_manifest",
    "preferred_preflight_profile": "marketplace_submission"
  },
  "bundle": {
    "skills": true,
    "mcp_servers": true,
    "apps": false
  }
}
```

This is intentionally an illustrative contract, not a full implementation dump.
The important thing is the boundary between package truth and listing truth.

## Source Descriptor Contract

The source descriptor identifies where the package comes from.

It should be narrow and typed.

Recommended initial source kinds:

- `local_path`
- `git_ref`
- `archive`
- `registry_package`

### Why keep source kinds narrow

The marketplace contract should identify package origin without embedding a full
package manager inside the catalog schema.

That means:

- enough data to locate the package
- enough data to attach provenance and integrity metadata later
- no attempt to encode all transport-specific installation logic into the root
  schema

### Illustrative source shapes

Local path:

```json
{
  "kind": "local_path",
  "path": "./plugins/gmail-sync"
}
```

Git ref:

```json
{
  "kind": "git_ref",
  "url": "https://github.com/example/gmail-sync",
  "ref": "v0.4.2"
}
```

Archive:

```json
{
  "kind": "archive",
  "url": "https://example.invalid/releases/gmail-sync-v0.4.2.tar.gz",
  "sha256": "..."
}
```

Registry package:

```json
{
  "kind": "registry_package",
  "registry": "loongclaw-registry",
  "package": "gmail-sync",
  "version": "0.4.2"
}
```

## Install Policy Vocabulary

Install policy should answer one question only:

- how does this listing expect installation to happen?

Recommended initial values:

- `not_installable`
- `operator_install`
- `available`
- `installed_by_default`

### Semantics

- `not_installable`
  - listed for awareness or migration context only
  - the catalog does not permit normal installation from this entry
- `operator_install`
  - installation is possible, but only through explicit operator action
- `available`
  - installable through the normal product/operator path
- `installed_by_default`
  - intended to land as part of a default bundle or curated baseline

### Why both `operator_install` and `available`

LoongClaw is governance-heavy.
Some listings may be valid and curated while still requiring explicit operator
consent before installation.
That nuance should live in the marketplace contract instead of being treated as
an implementation detail.

## Authentication Policy Vocabulary

Authentication policy should answer:

- when does the listing expect user or operator authentication to matter?

Recommended initial values:

- `none`
- `on_install`
- `on_first_use`

### Semantics

- `none`
  - no extra authentication is expected for normal package use
- `on_install`
  - installation should guide the operator through setup or auth immediately
- `on_first_use`
  - installation may remain lightweight, but real use should trigger setup or
    auth guidance before execution

This is intentionally smaller than a full auth state machine.
Detailed setup remains package-manifest and onboarding territory.

## Interface Metadata Contract

Interface metadata should be strict enough for product and marketplace tooling.

Recommended fields:

- `display_name`
- `short_description`
- `long_description`
- `developer_name`
- `category`
- `capabilities`
- `website_url`
- `privacy_policy_url`
- `terms_of_service_url`
- `brand_color`
- `icon`
- `logo`
- `screenshots`
- `starter_prompts`

### Why allow marketplace-level interface metadata

A package manifest may remain intentionally runtime-focused.
A curated catalog may still need:

- better summaries
- translated display copy
- category normalization
- operator-facing install notes

That is acceptable as long as:

- the marketplace layer only changes display and listing semantics
- runtime identity and activation truth still come from the package contract

## Bundle Composition Contract

A listing should be able to advertise which product surfaces the package
contributes to.

Recommended initial fields:

- `skills`
- `mcp_servers`
- `apps`

These may be booleans or summary counts in an implementation-specific shape,
provided the semantics remain descriptive.

### Why this matters

Codex's strongest packaging lesson is that plugins are often bundles of product
surfaces, not only executable extensions.
LoongClaw should preserve that lesson without collapsing bundle metadata into
activation authority.

## Trust And Review Contract

The marketplace layer may expose trust and review posture, but it must remain
advisory to runtime activation.

Recommended fields:

- `review_tier`
- `publisher`
- `review_notes`
- `attestation_required`

### Suggested `review_tier` vocabulary

- `unreviewed`
- `reviewed`
- `official`

This is compatible with the broader trust-tier and provenance direction, but it
is not itself the runtime trust gate. Manifest `trust_tier` and package
provenance remain package-sourced inputs. Marketplace `review_tier` is curated
listing metadata. Marketplace `attestation_required` is advisory listing
metadata only, not a runtime attestation gate. Actual attestation validity and
enforcement remain preflight, activation, and runtime-governance concerns.

## Compatibility Projection Contract

A marketplace listing may include descriptive compatibility metadata so
operators and UIs can understand the likely posture before install.

Recommended fields:

- `declared_dialect`
- `declared_compatibility_mode`
- `preferred_preflight_profile`
- `expected_bundle_kind`

### Important rule

These fields are projections, not runtime overrides.

If they disagree with package or preflight truth:

- the package and governance layers win
- the marketplace entry is treated as stale or invalid metadata
- the host should surface that mismatch instead of silently trusting the listing

## Precedence Rules

The system should keep precedence explicit.

### Package contract wins for

- `plugin_id`
- setup requirements
- declared capabilities
- slot claims
- host compatibility declarations
- dialect normalization truth
- activation and attestation truth

### Marketplace contract may define or override only

- display and discovery metadata
- install policy
- authentication timing
- curation and review metadata
- listing-specific visibility hints
- narrowly typed host-version or product gating for listing visibility

### Host governance still wins for

- whether a package is allowed to install in the current environment
- whether a package passes `plugin_preflight`
- whether a package may activate under the current bridge matrix
- whether a loaded provider's attestation is still valid

This is the most important marketplace rule.
It prevents catalog metadata from becoming shadow authority.

## Relationship To `plugin_preflight`

The marketplace contract should explicitly align with the existing
`marketplace_submission` preflight lane.

That means:

- marketplace publication workflows should evaluate package truth through
  `plugin_preflight`
- marketplace tooling should consume the same diagnostics and action plans that
  operator tooling already sees
- marketplace catalogs should not ship their own private policy parser as the
  primary source of truth

### Practical implication

A curated catalog entry may be listed before install, but publication and
promotion decisions should still be explainable through the same:

- diagnostics
- remediation classes
- recommended actions
- policy provenance

that the host already emits.

## Relationship To Imported Registries And Foreign Ecosystems

The marketplace contract should support imported ecosystems without claiming
that imported packages are native.

That means a listing may describe:

- a native LoongClaw package
- an OpenClaw-compatible package
- a future imported foreign package

while still preserving:

- package provenance
- dialect identity
- compatibility posture

The catalog is therefore a listing surface, not a flattening surface.
It should never erase the distinction between native and foreign packages.

## Suggested Validation Standard

Any implementation that claims conformance with this contract should verify:

- strict schema parsing for catalog roots and entries
- stable precedence between package truth and marketplace truth
- rejection or warning when marketplace projections contradict package truth
- preservation of foreign-dialect identity in imported listings
- alignment with `plugin_preflight` and `marketplace_submission` workflows
- zero paths where marketplace metadata bypasses activation governance

For doc-only changes, the minimum repository checks should include:

- `cargo fmt --all -- --check`
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`

## Relationship To Existing RFCs And Issues

This contract should be treated as a sibling layer to:

- `#522` manifest-first plugin package contract
- `#426` plugin SDK crate RFC
- the OpenClaw compatibility contract

The package contract defines runtime-facing package truth.
The SDK RFC defines author-facing tooling layers.
This marketplace contract defines how listed packages are distributed,
curated, and presented without redefining runtime truth.

## Non-Goals

This contract does not:

- implement a marketplace backend
- define package signature verification in detail
- define ranking or recommendation algorithms
- replace host governance with catalog metadata
- guarantee that every listed package is activation-ready on every host
- define UI rendering details for every product surface

Those are follow-on implementation concerns.

## Future Direction

The long-term target is a marketplace layer that remains:

- aligned with package and compatibility truth
- explicit about install and auth posture
- compatible with native and foreign package ecosystems
- reusable across local catalogs, imported registries, and remote products
- safe because it stays descriptive rather than authoritative

That gives LoongClaw the missing ecosystem layer between package manifests and
future marketplace products, without weakening the kernel-first architecture.
