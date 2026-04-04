# Channel Bridge Managed Discovery Remediation Design

## Scope

This slice deepens managed bridge discovery for plugin-backed `weixin`,
`qqbot`, and `onebot` surfaces by making two operator-facing gaps explicit:

- remediation guidance when discovered bridge manifests are incomplete or
  incompatible
- ambiguity diagnostics when the managed install root contains more than one
  ready-compatible bridge plugin for the same surface

The immediate scope is:

- enrich discovered plugin summaries with setup-guidance facts that already
  exist in the manifest
- model whether a surface has zero, one, or multiple ready-compatible managed
  bridge plugins
- surface those facts through `loongclaw doctor`, `doctor` next steps,
  `loongclaw channels`, runtime snapshot text, and `channels --json`

This slice does not:

- add a plugin-selection config mechanism
- add a new manifest schema
- probe external bridge process liveness
- change the meaning of the existing operation-level bridge contract checks

## Problem Statement

The previous slice closed the visibility gap between static bridge contracts and
managed plugin inventory. Operators can now see whether managed discovery found
matching bridge manifests for `weixin`, `qqbot`, and `onebot`.

That still leaves two important gaps unresolved:

- if discovery finds an incomplete or incompatible manifest, operator output
  still stops too early and does not turn the discovered facts into a clear
  repair path
- if discovery finds multiple ready-compatible manifests for the same surface,
  current output still reports a generic pass because at least one compatible
  plugin exists, even though the managed install root is now ambiguous

The root cause is that the discovery model currently records only coarse
  compatibility counts and low-level issue fields. It does not project setup
  guidance into the typed discovery summary, and daemon still treats
  `compatible_plugins > 0` as an unconditional pass.

## Constraints

The design must preserve the current architecture split:

- `kernel` owns manifest scanning and bridge translation
- `app` owns typed discovery aggregation
- `daemon` owns doctor policy, next-step guidance, and rendering

It also has to preserve two boundaries:

- app discovery structs should hold reusable facts, not CLI-specific strings
- daemon should build remediation and ambiguity guidance from those facts rather
  than duplicating app validation logic

The design must stay generic across plugin-backed bridge surfaces and must not
introduce `weixin` / `qqbot` / `onebot` special cases in doctor policy.

## Approaches Considered

### 1. Add daemon-only remediation strings

Rejected because it would force daemon to reconstruct setup guidance from free
form detail strings and would duplicate compatibility logic outside the app
inventory seam.

That path would also make `channels --json` stay structurally weak because the
new guidance would exist only in `doctor`.

### 2. Add a full plugin-selection policy/config surface now

Rejected because it widens the problem from diagnostics into operator policy.

The immediate operator need is truthful ambiguity detection and repair guidance,
not a new configuration mechanism for picking a preferred plugin.

### 3. Enrich discovery facts in app and let daemon derive guidance

Chosen because it keeps the data flow clean:

- app can expose existing manifest setup facts without inventing new meaning
- daemon can reuse those facts to render text and next steps
- ambiguity can stay a typed discovery outcome instead of an implicit
  interpretation of counts

This is the smallest correct step that closes the operator gap without pulling
selection policy into scope.

## Chosen Design

### 1. Enrich discovered plugin summaries with setup guidance

`ChannelDiscoveredPluginBridge` will carry the setup facts that are already
available in `PluginManifest.setup`:

- `required_env_vars`
- `recommended_env_vars`
- `required_config_keys`
- `default_env_var`
- `docs_urls`
- `remediation`

These are reusable discovery facts, not presentation strings, so they belong in
app-owned inventory data.

### 2. Add a typed compatibility selection outcome

`ChannelPluginBridgeDiscovery` will record whether ready-compatible managed
plugins for a surface are:

- absent
- unique
- ambiguous

This prevents daemon from inferring a pass state from `compatible_plugins > 0`
when discovery actually found multiple equally-ready plugins.

### 3. Keep ambiguity semantics conservative

Ambiguity is defined only for ready-compatible managed plugins.

That means:

- zero ready-compatible plugins is not ambiguous; it is a compatibility gap
- one ready-compatible plugin is healthy
- two or more ready-compatible plugins for the same surface is ambiguous

Incomplete and incompatible plugins still matter for remediation, but they do
not create ambiguity by themselves.

### 4. Let daemon build remediation guidance from typed facts

Daemon will keep rendering the discovery summary, but it will add:

- clearer `doctor` detail text for ambiguous surfaces
- `doctor` next steps derived from missing fields, setup requirements, docs
  URLs, and manifest remediation text
- text output that exposes the new ambiguity status and setup guidance

This keeps the responsibility split clean:

- app says what discovery found
- daemon says how an operator should react

### 5. Preserve existing bridge contract checks

The existing operation-level checks stay unchanged.

The new logic applies only to the surface-level managed discovery check and the
derived next steps. This avoids semantic drift in the checks added by the
previous bridge diagnostics slice.

## Why This Design

This is the smallest correct next step because it turns already-discovered facts
into operator-usable diagnostics without introducing new policy:

- manifest setup guidance already exists
- managed ambiguity already exists implicitly when multiple compatible plugins
  are found
- current discovery output already has the right app/daemon seam

The chosen design avoids hardcoding by letting generic manifest/setup facts
drive the operator experience:

- missing fields become repair steps
- plugin setup docs become review links
- manifest remediation text becomes explicit operator guidance
- multiple compatible plugins become a typed ambiguity state instead of a hidden
  edge case

That produces materially better `weixin` / `qqbot` / `onebot` support while
keeping the implementation small, typed, and reusable for future plugin-backed
bridge surfaces.
